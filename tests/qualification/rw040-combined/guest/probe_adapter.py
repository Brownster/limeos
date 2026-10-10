"""Strict receiver for the unchanged integrator libtest probe; no authority API."""

import json
import select
import time

PREFIX = b"LIMEOS_RW040_PROBE_JSON="
ARGV = ["--exact", "qualification_probe", "--nocapture", "--test-threads=1"]
SOURCE_SHA256 = "3929400eb457937923311206201e4ef8ce2b935afc685ec7e81b27e557699b9c"
CASES = {
    "contract",
    "engine",
    "engine-processes",
    "engine-sources",
    "storage",
    "engine-dependencies",
    "combined",
}
MAX_EVENT = 256 << 10
MAX_NOISE = 32 << 10
MAX_OUTPUT = 2 * (MAX_EVENT + len(PREFIX) + 1) + MAX_NOISE
SUMMARY = b"test result: ok. 1 passed; 0 failed; 0 ignored;"


def pidfd_exited(fd: int) -> bool:
    poller = select.poll()
    poller.register(fd, select.POLLIN)
    events = poller.poll(0)
    if any(flags & (select.POLLNVAL | select.POLLERR) for _, flags in events):
        raise OSError("invalid sampled process identity")
    return any(flags & select.POLLIN for _, flags in events)


def unique_object(pairs):
    value = {}
    for key, item in pairs:
        if key in value:
            raise ValueError("duplicate JSON key")
        value[key] = item
    return value


def invalid_constant(value):
    raise ValueError(f"non-JSON number: {value}")


def probe_environment(case: str, pause_ms: int = 0) -> dict[str, str]:
    if case not in CASES or type(pause_ms) is not int or not 0 <= pause_ms <= 5000:
        raise ValueError("invalid trusted probe selection")
    if case == "engine-dependencies":
        raise ValueError("a genuine protected storage contract is not supplied")
    return {
        "LIMEOS_RW040_PROBE_CASE": case,
        "LIMEOS_RW040_PROBE_SOCKET": "/run/docker.sock",
        "LIMEOS_RW040_PROBE_PAUSE_MS": str(pause_ms),
    }


class Events:
    """Bound output before line decoding; a collected event is provisional."""

    def __init__(self, case: str, collected=None):
        if case not in CASES:
            raise ValueError("unknown producer case")
        self.case = case
        self.collected = collected
        self.buffer = bytearray()
        self.events = []
        self.timestamps = []
        self.noise = bytearray()
        self.total = 0
        self.raw = bytearray()
        self.started = time.monotonic()

    def feed(self, data: bytes):
        self.total += len(data)
        if self.total > MAX_OUTPUT:
            raise ValueError("probe output exceeds its bound")
        self.raw.extend(data)
        self.buffer.extend(data)
        while b"\n" in self.buffer:
            line, _, rest = self.buffer.partition(b"\n")
            self.buffer = bytearray(rest)
            self._line(bytes(line))
        if len(self.buffer) > MAX_EVENT + len(PREFIX):
            raise ValueError("probe line exceeds its bound")

    def _line(self, line: bytes):
        if not line.startswith(PREFIX):
            if PREFIX in line:
                raise ValueError("malformed probe event framing")
            if len(self.noise) + len(line) + 1 > MAX_NOISE:
                raise ValueError("probe diagnostic output exceeds its bound")
            self.noise.extend(line + b"\n")
            return
        body = line[len(PREFIX) :]
        if len(body) > MAX_EVENT or len(self.events) >= 2:
            raise ValueError("probe event count or body exceeds its bound")
        if self.events and self.events[-1]["phase"] == "final":
            raise ValueError("event after final probe event")
        try:
            value = json.loads(
                body.decode("utf-8"),
                object_pairs_hook=unique_object,
                parse_constant=invalid_constant,
            )
        except (UnicodeError, json.JSONDecodeError, RecursionError) as error:
            raise ValueError("malformed probe JSON") from error
        fields = {
            "version",
            "case",
            "phase",
            "status",
            "stage",
            "error",
            "effective_uid",
            "observations",
        }
        if (
            not isinstance(value, dict)
            or set(value) != fields
            or type(value["version"]) is not int
            or value["version"] != 1
            or value["case"] != self.case
            or type(value["effective_uid"]) is not int
            or value["effective_uid"] != 0
            or value["phase"] not in ("collected", "final")
            or value["status"] not in ("ok", "refused", "invalid_input", "blocked")
            or not isinstance(value["stage"], str)
            or len(value["stage"]) > 64
            or not isinstance(value["observations"], dict)
        ):
            raise ValueError("invalid probe event fields")
        stages = {
            "prerequisites",
            "input",
            "engine.collect",
            "processes.collect",
            "sources.collect",
            "dependencies.collect",
            "contract.read",
            "storage.collect",
            "storage.collected",
            "owners.collected",
            "engine.before_revalidate",
            "engine.after_revalidate",
            "owner.revalidate",
            "storage.revalidate",
            "owners.revalidated",
            "storage.revalidated",
        }
        if value["stage"] not in stages:
            raise ValueError("unknown producer stage")
        if value["phase"] == "collected":
            expected_stage = (
                "storage.collected" if self.case == "storage" else "owners.collected"
            )
            if (
                self.events
                or value["status"] != "ok"
                or value["stage"] != expected_stage
                or value["error"] is not None
            ):
                raise ValueError("invalid collected event")
            if self.case in ("contract", "combined"):
                raise ValueError("prerequisite cases cannot collect owners")
        elif value["status"] == "refused":
            error = value["error"]
            if (
                not isinstance(error, dict)
                or set(error) != {"library", "code"}
                or error["library"] not in ("engine", "storage")
                or error["code"] is None
                or value["observations"]
            ):
                raise ValueError("invalid typed refusal")
        elif value["status"] in ("ok", "blocked") and value["error"] is not None:
            raise ValueError("unexpected probe error")
        if value["status"] == "blocked" and (
            self.case != "combined" or value["stage"] != "prerequisites"
        ):
            raise ValueError("unsupported blocked result")
        if self.case == "contract" and (
            value["phase"] != "final"
            or value["status"] != "ok"
            or value["stage"] != "prerequisites"
        ):
            raise ValueError("invalid contract prerequisite result")
        if value["status"] == "ok":
            required = {
                "contract": {
                    "available_cases",
                    "combined_worker",
                    "standalone_supervision",
                },
                "engine": {"engine"},
                "engine-processes": {"engine", "processes"},
                "engine-sources": {"engine", "sources"},
                "storage": {"storage"},
                "engine-dependencies": {"engine", "dependencies"},
            }.get(self.case)
            if required is None or set(value["observations"]) != required:
                raise ValueError("incomplete selected-owner observations")
            if self.case != "contract" and any(
                not isinstance(item, dict) for item in value["observations"].values()
            ):
                raise ValueError("selected-owner observations must be objects")
        if (
            value["phase"] == "final"
            and value["status"] == "ok"
            and self.case not in ("contract", "combined")
        ):
            expected_stage = (
                "storage.revalidated"
                if self.case == "storage"
                else "owners.revalidated"
            )
            if not self.events or value["stage"] != expected_stage:
                raise ValueError("success without retained collection/revalidation")
        self.events.append(value)
        self.timestamps.append(
            {
                "phase": value["phase"],
                "monotonic": time.monotonic(),
                "unix": time.time(),
            }
        )
        if value["phase"] == "collected" and self.collected is not None:
            self.collected(value)

    def finish(self, exit_code: int) -> dict:
        if self.buffer:
            raise ValueError("truncated probe line")
        if exit_code != 0 or self.noise.count(SUMMARY) != 1:
            raise ValueError("probe/libtest execution did not succeed")
        if not self.events or self.events[-1]["phase"] != "final":
            raise ValueError("probe lacks one final event")
        final = self.events[-1]
        if (
            self.case not in ("contract", "combined")
            and not 0 <= self.timestamps[-1]["monotonic"] - self.started <= 5
        ):
            raise ValueError("probe final delivery exceeds the conservative lifetime")
        if final["status"] == "invalid_input":
            raise ValueError("trusted probe configuration refused")
        engine = final["observations"].get("engine")
        if final["status"] == "ok" and engine is not None:
            observed = engine["declarations"]["observed_at"]
            before = self.events[0]["observations"]["engine"]["declarations"][
                "observed_at"
            ]
            if (
                type(observed) is not int
                or observed != before
                or not 0 <= self.timestamps[-1]["unix"] - observed <= 5
            ):
                raise ValueError("expired or renewed Engine observation at delivery")
        return {
            "events": self.events,
            "received": self.timestamps,
            "final_status": final["status"],
            "exit": exit_code,
        }
