"""Bounded read-only observations of the disposable guest's reader service."""

import hashlib
import os
import threading
from pathlib import Path

from p04_automation_guest import disposable_guard, require

GROUP = Path("/sys/fs/cgroup/system.slice/limeos-storage-reader.service")


def counters(text):
    return {
        key: int(value) for line in text.splitlines() for key, value in [line.split()]
    }


def process_start(pid):
    raw = (Path("/proc") / str(pid) / "stat").read_text()
    return int(raw[raw.rindex(")") + 1 :].split()[19])


class ReaderResources:
    def __init__(self, binaries):
        self.binaries = binaries
        self.stop = threading.Event()
        self.thread = None
        self.value = {
            "samples": 0,
            "memory_peak": 0,
            "tasks_maximum": 0,
            "sampled_fd_maximum": 0,
            "processes": {},
            "errors": [],
        }
        self.images = {}

    def start(self):
        disposable_guard()
        require(self.thread is None, "reader observer already started")
        self.thread = threading.Thread(target=self.observe, daemon=True)
        self.thread.start()

    def sample(self):
        values = {
            name: (GROUP / name).read_text().strip()
            for name in (
                "memory.current",
                "memory.peak",
                "memory.max",
                "memory.events",
                "pids.current",
                "pids.max",
                "pids.events",
            )
        }
        require(
            values["memory.max"] == str(64 * 1024**2) and values["pids.max"] == "16",
            "reader cgroup ceiling differs",
        )
        events = counters(values["memory.events"])
        require(
            all(events[key] == 0 for key in ("max", "oom", "oom_kill")),
            "reader memory event occurred",
        )
        require(
            counters(values["pids.events"])["max"] == 0, "reader task event occurred"
        )
        self.value["samples"] += 1
        self.value["memory_peak"] = max(
            self.value["memory_peak"], int(values["memory.peak"])
        )
        self.value["tasks_maximum"] = max(
            self.value["tasks_maximum"], int(values["pids.current"])
        )
        self.value["last_cgroup"] = values
        pids = (GROUP / "cgroup.procs").read_text().split()
        require(len(pids) <= 16, "reader process count exceeds bound")
        for pid in pids:
            try:
                self.process(int(pid))
            except (FileNotFoundError, ProcessLookupError):
                pass

    def process(self, pid):
        proc = Path("/proc") / str(pid)
        start = process_start(pid)
        image = os.readlink(proc / "exe")
        status = {
            key: value.strip()
            for line in (proc / "status").read_text().splitlines()
            if ":" in line
            for key, value in [line.split(":", 1)]
            if key
            in (
                "PPid",
                "Uid",
                "CapEff",
                "CapPrm",
                "CapBnd",
                "NoNewPrivs",
                "Threads",
                "VmHWM",
            )
        }
        with os.scandir(proc / "fd") as entries:
            count = sum(1 for _ in entries)
        require(count <= 256, "reader FD observation exceeds bound")
        self.value["sampled_fd_maximum"] = max(self.value["sampled_fd_maximum"], count)
        if (
            image in ("/usr/lib/systemd/systemd", "/lib/systemd/systemd")
            and status["PPid"] == "1"
        ):
            return  # systemd's pre-exec launcher, not a worker image witness
        command = (proc / "cmdline").read_bytes()
        require(len(command) <= 4096, "reader command observation exceeds bound")
        role_argument = command.rstrip(b"\0").split(b"\0")[-1].decode(errors="replace")
        identity = f"{pid}:{start}:{image}"
        digest = self.images.get(identity)
        if digest is None:
            with (proc / "exe").open("rb") as stream:
                digest = hashlib.file_digest(stream, "sha256").hexdigest()
        if process_start(pid) != start or os.readlink(proc / "exe") != image:
            return
        self.images[identity] = digest
        require(len(self.images) <= 256, "reader image observations exceed bound")
        if (
            image == "/usr/lib/limeos/limeos-executor"
            and digest == self.binaries["limeos-executor"]
        ):
            role = "manager"
        elif (
            image.startswith("/memfd:")
            and digest == self.binaries["limeos-stack-read-worker"]
            and role_argument
            in ("guardian", "worker", "health-guardian", "health-worker")
        ):
            role = role_argument
        elif image.startswith("/memfd:") and digest == self.binaries["limeos-executor"]:
            role = "storage-worker"
        else:
            raise ValueError("unexpected reader process image")
        require(
            all(
                status[key] == "0000000000000000"
                for key in ("CapEff", "CapPrm", "CapBnd")
            )
            and status["NoNewPrivs"] == "1",
            "reader process confinement differs",
        )
        self.value["processes"][identity] = {
            "pid": pid,
            "start_ticks": start,
            "image": image,
            "sha256": digest,
            "role": role,
            "status": status,
            "sampled_fds": count,
        }

    def observe(self):
        while not self.stop.wait(0.01):
            try:
                self.sample()
            except (FileNotFoundError, ProcessLookupError):
                pass  # startup/restart may remove the cgroup or a child
            except (OSError, ValueError, KeyError) as error:
                self.value["errors"].append(type(error).__name__)
                if len(self.value["errors"]) >= 16:
                    return

    def finish(self, *, validate=True):
        self.stop.set()
        if self.thread is not None:
            self.thread.join(timeout=2)
            require(not self.thread.is_alive(), "reader observer did not stop")
        if validate:
            require(
                not self.value["errors"] and self.value["samples"] > 0,
                "reader resource observation failed",
            )
            require(
                self.value["memory_peak"] < 64 * 1024**2
                and self.value["tasks_maximum"] <= 16,
                "reader observed resource ceiling exceeded",
            )
            roles = {row["role"] for row in self.value["processes"].values()}
            require(
                {"manager", "guardian", "worker"}.issubset(roles),
                "reader image role observations incomplete",
            )
        return self.value
