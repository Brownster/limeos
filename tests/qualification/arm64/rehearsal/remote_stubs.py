#!/usr/bin/env python3
"""Commands that exist on wybie, imitated for the Pi night rehearsal.

Every stub reads and writes plain files under the rehearsal control directory
(`ctl`), seeded from the real snapshot captured on wybie on 2026-11-07
(`fixtures/wybie-host-pre.json`) and the real apt simulation from 2026-10-13
(`fixtures/apt-install-simulation.txt`). Output formats are the real ones: the stubs
reject any `docker --format` placeholder or `dpkg-query -f` field they do not know,
so a harness that asks for a format wybie would not understand fails here too.

Each stub appends its argv to `<ctl>/calls.jsonl`; the rehearsal asserts on it (for
example that nothing ever starts, stops or restarts a container).
"""

import fcntl
import json
import os
import re
import signal
import subprocess
import sys
import time
from pathlib import Path

HERE = Path(__file__).resolve().parent
CTL = None
GIB = 1024 * 1024
QEMU_REQUESTED = ("qemu-system-arm", "qemu-efi-aarch64", "qemu-utils")


# --------------------------------------------------------------------- plumbing
def state(name):
    return CTL / "state" / name


def locked(name):
    stream = (CTL / "state" / f"{name}.lock").open("a+")
    fcntl.flock(stream, fcntl.LOCK_EX)
    return stream


def load(name, default=None):
    try:
        return json.loads(state(name).read_text())
    except OSError:
        return default


def save(name, value):
    temp = state(name).with_suffix(".tmp")
    temp.write_text(json.dumps(value, indent=1))
    temp.replace(state(name))


def log_call(name, argv):
    line = json.dumps({"epoch": round(time.time(), 3), "cmd": name, "argv": argv}) + "\n"
    with (CTL / "calls.jsonl").open("a") as stream:
        stream.write(line)


def scenario():
    try:
        return json.loads((CTL / "scenario.json").read_text())
    except OSError:
        return {}


def fail(message, code=1):
    print(message, file=sys.stderr)
    return code


def unescape(text):
    return text.replace("\\t", "\t").replace("\\n", "\n")


# ----------------------------------------------------------------------- docker
def lookup(context, path):
    node = context
    for part in path.lstrip(".").split("."):
        if not isinstance(node, dict) or part not in node:
            raise KeyError(path)
        node = node[part]
    return node


def render(template, context):
    def sub(match):
        expression = match.group(1).strip()
        if expression.startswith("json "):
            return json.dumps(lookup(context, expression[5:].strip()), separators=(",", ":"))
        return str(lookup(context, expression))

    return re.sub(r"\{\{(.*?)\}\}", sub, unescape(template))


def containers():
    return load("containers.json", [])


def docker_ps_context(row, trunc):
    return {
        "ID": row["id"] if not trunc else row["id"][:12],
        "Names": row["name"],
        "State": row["state"],
        "RunningFor": row["running_for"],
        "Status": row["status"],
        "Image": "image",
    }


def docker_inspect_context(row):
    return {
        "Id": row["id"],
        "Name": "/" + row["name"],
        "RestartCount": row["restarts"],
        "State": {"Status": row["state"], "StartedAt": row["started_at"]},
    }


def stop_container(name, actor="external client", signal_name="15"):
    """What an unidentified client doing `docker stop` does to the daemon's state."""
    lock = locked("containers")
    try:
        rows = containers()
        for row in rows:
            if row["name"] == name and row["state"] == "running":
                row["state"] = "exited"
                row["status"] = "Exited (143) Less than a second ago"
                with (CTL / "events.jsonl").open("a") as stream:
                    for action in ("kill", "die", "stop"):
                        stream.write(
                            json.dumps(
                                {
                                    "Time": int(time.time()),
                                    "Action": action,
                                    "Actor": {
                                        "Attributes": {
                                            "image": "image",
                                            "name": name,
                                            "signal": signal_name,
                                            "actor": actor,
                                        }
                                    },
                                }
                            )
                            + "\n"
                        )
                save("containers.json", rows)
                return True
        return False
    finally:
        lock.close()


def docker(argv):
    if scenario().get("docker_denied"):
        return fail(
            "permission denied while trying to connect to the Docker daemon socket at "
            "unix:///var/run/docker.sock: Get \"http://%2Fvar%2Frun%2Fdocker.sock/v1.47/containers/json\": "
            "dial unix /var/run/docker.sock: connect: permission denied"
        )
    if not argv:
        return fail("docker: a subcommand is required")
    command, rest = argv[0], argv[1:]
    rows = containers()
    template = None
    flags = set()
    positional = []
    i = 0
    options_with_value = {"--format", "-f", "--since", "--until", "--filter"}
    values = {}
    while i < len(rest):
        arg = rest[i]
        if arg in options_with_value:
            values[arg] = rest[i + 1]
            i += 2
            continue
        if arg.startswith("-"):
            flags.add(arg)
        else:
            positional.append(arg)
        i += 1
    template = values.get("--format") or values.get("-f")
    try:
        if command == "ps":
            show_all = bool({"-a", "-aq", "-qa", "--all"} & flags)
            quiet = bool({"-q", "-aq", "-qa", "--quiet"} & flags)
            trunc = "--no-trunc" not in flags
            lines = []
            for row in rows:
                if not show_all and row["state"] != "running":
                    continue
                if quiet:
                    lines.append(row["id"][:12] if trunc else row["id"])
                elif template:
                    lines.append(render(template, docker_ps_context(row, trunc)))
                else:
                    lines.append(f"{row['id'][:12]}  image  {row['name']}  {row['status']}")
            print("\n".join(lines))
            return 0
        if command == "inspect":
            if not template:
                return fail("rehearsal docker inspect needs --format")
            lines = []
            for ident in positional:
                match = [r for r in rows if r["id"].startswith(ident) or r["name"] == ident]
                if len(match) != 1:
                    return fail(f"Error: No such object: {ident}")
                lines.append(render(template, docker_inspect_context(match[0])))
            print("\n".join(lines))
            return 0
        if command == "events":
            since = int(values.get("--since", 0))
            until = int(values.get("--until", 2**62))
            lines = []
            try:
                events = [json.loads(l) for l in (CTL / "events.jsonl").read_text().splitlines()]
            except OSError:
                events = []
            for event in events:
                if since <= event["Time"] <= until:
                    lines.append(render(template, event))
            print("\n".join(lines))
            return 0
        if command in ("stop", "kill"):
            ok = all(stop_container(name) for name in positional)
            print("\n".join(positional))
            return 0 if ok else 1
        return fail(f"docker: unsupported rehearsal subcommand {command}")
    except KeyError as error:
        return fail(f"template parsing error: can't evaluate field {error}")


# ------------------------------------------------------------------ dpkg and apt
def packages():
    rows = {}
    for line in state("packages.tsv").read_text().splitlines():
        key, _, version = line.partition("\t")
        if key:
            rows[key] = version
    return rows


def save_packages(rows):
    state("packages.tsv").write_text(
        "".join(f"{k}\t{rows[k]}\n" for k in sorted(rows, key=lambda k: (k.split(":")[0], k)))
    )


def bare(rows):
    return {k.split(":")[0]: k for k in rows}


def dpkg_query(argv):
    fmt, names, i = None, [], 0
    while i < len(argv):
        if argv[i] in ("-W", "--show"):
            i += 1
        elif argv[i] in ("-f", "--showformat"):
            fmt = argv[i + 1]
            i += 2
        else:
            names.append(argv[i])
            i += 1
    fmt = unescape(fmt or "${Package}\t${Version}\n")
    rows = packages()
    known = bare(rows)
    chosen = names or sorted(known)
    out, missing = [], []
    for name in chosen:
        if name not in known:
            missing.append(name)
            continue
        key = known[name]
        package, _, arch = key.partition(":")

        def field(match):
            table = {
                "Package": package,
                "Architecture": arch,
                "Version": rows[key],
                "Status": "install ok installed",
            }
            if match.group(1) not in table:
                raise KeyError(match.group(1))
            return table[match.group(1)]

        try:
            out.append(re.sub(r"\$\{([A-Za-z-]+)\}", field, fmt))
        except KeyError as error:
            return fail(f"dpkg-query: error: unknown field {error}")
    sys.stdout.write("".join(out))
    for name in missing:
        print(f"dpkg-query: no packages found matching {name}", file=sys.stderr)
    return 1 if missing else 0


def apt_mark(argv):
    if argv[:1] == ["showmanual"]:
        sys.stdout.write(state("manual.txt").read_text())
        return 0
    return fail("apt-mark: unsupported rehearsal subcommand")


def catalog():
    items = []
    for line in (HERE / "fixtures/apt-install-simulation.txt").read_text().splitlines():
        match = re.match(r"Inst (\S+) \((\S+) .*\[(\w+)\]\)$", line)
        if match:
            items.append({"name": match[1], "version": match[2], "arch": match[3], "line": line})
    return items


def apt_get(argv):
    flags = [a for a in argv if a.startswith("-")]
    words = [a for a in argv if not a.startswith("-")]
    simulate = "-s" in flags or "--simulate" in flags
    if not words:
        return fail("E: Invalid operation")
    verb, names = words[0], words[1:]
    if os.geteuid() == 0:
        pass
    rows = packages()
    known = bare(rows)
    if verb == "install":
        if "--no-install-recommends" not in flags:
            return fail("rehearsal: install without --no-install-recommends")
        missing_request = [n for n in names if n not in known]
        needed = [c for c in catalog() if c["name"] not in known] if missing_request else []
        if simulate:
            print("Reading package lists...\nBuilding dependency tree...\nReading state information...")
            for name in names:
                if name in known:
                    print(f"{name} is already the newest version ({rows[known[name]]}).")
            if not needed:
                print("0 upgraded, 0 newly installed, 0 to remove and 62 not upgraded.")
            if scenario().get("apt_upgrade"):
                print(f"Inst libc6 [2.36-9+rpt2+deb12u9] (2.36-9+rpt2+deb12u10 Debian:12.15/oldstable [arm64])")
            for item in needed:
                print(item["line"])
            for item in needed:
                print(item["line"].replace("Inst ", "Conf ", 1))
            return 0
        if scenario().get("apt_install_fail"):
            for item in needed[: len(needed) // 2]:
                rows[f"{item['name']}:{item['arch']}"] = item["version"]
            save_packages(rows)
            print("E: Sub-process /usr/bin/dpkg returned an error code (1)", file=sys.stderr)
            return 100
        for item in needed:
            rows[f"{item['name']}:{item['arch']}"] = item["version"]
            print(f"Setting up {item['name']} ({item['version']}) ...")
        manual = set(state("manual.txt").read_text().split())
        manual |= set(n for n in names)
        state("manual.txt").write_text("".join(f"{n}\n" for n in sorted(manual)))
        save_packages(rows)
        return 0
    if verb == "purge":
        targets = []
        for name in names:
            plain = name.split(":")[0]
            if plain not in known:
                return fail(f"E: Unable to locate package {name}")
            targets.append(known[plain])
        if simulate:
            print("Reading package lists...\nBuilding dependency tree...\nReading state information...")
            for key in targets:
                print(f"Purg {key.split(':')[0]} [{rows[key]}]")
            if scenario().get("purge_collateral"):
                print(f"Purg {scenario()['purge_collateral']} [1.0]")
            return 0
        if "-y" not in flags:
            return fail("rehearsal: purge without -y would prompt")
        if scenario().get("purge_fail"):
            return fail("E: Sub-process /usr/bin/dpkg returned an error code (1)", 100)
        for key in targets:
            print(f"Removing {key} ({rows.pop(key, '')}) ...")
        manual = set(state("manual.txt").read_text().split())
        manual -= {k.split(":")[0] for k in targets}
        state("manual.txt").write_text("".join(f"{n}\n" for n in sorted(manual)))
        save_packages(rows)
        return 0
    return fail(f"E: Invalid operation {verb}")


# ------------------------------------------------------------------ small system
def uname(argv):
    if "-m" in argv:
        print("aarch64")
    elif "-a" in argv:
        print(state("uname.txt").read_text().strip())
    else:
        print("Linux")
    return 0


def free(argv):
    sys.stdout.write(state("free.txt").read_text())
    return 0


def getconf(argv):
    if argv == ["PAGESIZE"]:
        print(16384)  # wybie's kernel uses 16 KiB pages
        return 0
    return subprocess.run(["/usr/bin/getconf", *argv]).returncode


def systemctl(argv):
    if argv[:1] == ["list-units"]:
        sys.stdout.write(state("units.txt").read_text())
        return 0
    if argv and argv[0] in ("start", "stop", "restart", "enable", "disable", "mask", "reload"):
        print("Failed to connect to bus: rehearsal refuses service changes", file=sys.stderr)
        return 1
    print("0 loaded units listed.")
    return 0


def ss(argv):
    if "-tlnp" in argv or "-tlnp" in " ".join(argv):
        sys.stdout.write(state("listeners.txt").read_text())
        return 0
    print("Netid State Recv-Q Send-Q Local Address:Port Peer Address:Port Process")
    return 0


def journalctl(argv):
    print("-- No entries --")
    return 0


def sudo(argv):
    from transport import settings  # noqa: F401  (import check: same directory)

    rest, non_interactive = list(argv), False
    while rest and rest[0].startswith("-"):
        option = rest.pop(0)
        if option == "-n":
            non_interactive = True
        elif option == "--":
            break
        elif option in ("-u", "-g", "-h", "-p"):
            rest.pop(0)
    if not non_interactive:
        return fail("sudo: a terminal is required to read the password; either use the -S option to read from standard input or configure an askpass helper")
    if scenario().get("sudo_denied"):
        return fail("sudo: a password is required")
    while rest and re.fullmatch(r"[A-Za-z_][A-Za-z0-9_]*=.*", rest[0]):
        name, _, value = rest.pop(0).partition("=")
        os.environ[name] = value  # sudo VAR=value cmd, as the 2026-10 runs used
    if not rest:
        return fail("sudo: no command specified")

    def mapped(arg):
        def sub(match):
            name = match.group(1)
            if name == "python3":
                return str(CTL / "bin/rehearsal-python3")
            candidate = CTL / "bin" / name
            return str(candidate) if candidate.exists() else match.group(0)

        return re.sub(r"(?<![\w.\-/])/usr/bin/([A-Za-z0-9._+-]+)", sub, arg)

    argv = [mapped(a) for a in rest]
    try:
        os.execvp(argv[0], argv)
    except OSError as error:
        return fail(f"sudo: {argv[0]}: command not found ({error})", 1)


def install(argv):
    cleaned, i = [], 0
    while i < len(argv):
        if argv[i] in ("-o", "-g"):
            i += 2
            continue
        cleaned.append(argv[i])
        i += 1
    os.execv("/usr/bin/install", ["/usr/bin/install", *cleaned])


# ------------------------------------------------------------------------- qemu
def qemu_img(argv):
    if argv[:1] != ["create"]:
        return fail("qemu-img: unsupported rehearsal subcommand")
    backing, target, size, i = None, None, None, 1
    while i < len(argv):
        if argv[i] in ("-f", "-F"):
            i += 2
        elif argv[i] == "-b":
            backing = argv[i + 1]
            i += 2
        elif argv[i] == "-q":
            i += 1
        elif target is None:
            target = argv[i]
            i += 1
        else:
            size = argv[i]
            i += 1
    if backing and not Path(backing).is_file():
        return fail(f"qemu-img: {target}: Could not open '{backing}': No such file or directory")
    if not re.fullmatch(r"\d+[KMGT]", size or ""):
        return fail(f"qemu-img: Invalid image size specified: {size}")
    Path(target).write_bytes(b"QFI\xfb" + (backing or "").encode())
    return 0


def qemu(argv):
    def value(flag):
        return argv[argv.index(flag) + 1] if flag in argv else None

    name, memory, cpus, runas = value("-name"), value("-m"), value("-smp"), value("-runas")
    forward = re.search(r"hostfwd=tcp:127\.0\.0\.1:(\d+)-:22", " ".join(argv))
    cwd = Path.cwd()
    problems = []
    if not (name and name.startswith("limeos-arm64-")):
        problems.append("-name must carry the limeos-arm64- marker")
    if not (memory and 512 <= int(memory) <= 2048):
        problems.append("-m out of range")
    if not forward:
        problems.append("no loopback hostfwd for ssh")
    if runas is None:
        problems.append("-runas missing: QEMU would keep running as root")
    for needed in ("disk.qcow2", "vars.fd", "seed.iso", "console.log"):
        if not (cwd / needed).exists():
            problems.append(f"Could not open '{needed}'")
    if problems:
        return fail("qemu-system-aarch64: " + "; ".join(problems))
    port = forward.group(1)
    root = CTL / "guests" / port
    (root / "var/lib/cloud/instance").mkdir(parents=True, exist_ok=True)
    (root / "root").mkdir(exist_ok=True)
    (root / "opt").mkdir(exist_ok=True)
    (root / "qemu.pid").write_text(str(os.getpid()))
    record = {
        "epoch": round(time.time(), 3),
        "name": name,
        "memory": int(memory),
        "cpus": int(cpus),
        "port": int(port),
        "runas": runas,
        "cwd": str(cwd),
        "argv": argv,
    }
    with (CTL / "qemu-launches.jsonl").open("a") as stream:
        stream.write(json.dumps(record) + "\n")

    def finish(*_):
        try:
            (root / "qemu.pid").unlink()
        except OSError:
            pass
        with (cwd / "console.log").open("a") as console:
            console.write("reached target Power-Off\n")
        sys.exit(0)

    signal.signal(signal.SIGTERM, finish)
    settings = scenario().get("qemu", {})
    with (cwd / "console.log").open("a") as console:
        console.write(f"{name} booting\n")
    time.sleep(float(settings.get("boot_delay", 0)))
    (root / "var/lib/cloud/instance/boot-finished").write_text("done\n")
    started = time.time()
    while True:
        if settings.get("exit_after") and time.time() - started > settings["exit_after"]:
            finish()
        time.sleep(0.1)


# ------------------------------------------------------------------------ guest
def meminfo_lines(mem):
    return "".join(f"{k}: {v:>12} kB\n" for k, v in mem.items())


def render_proc(ctl):
    """Write /proc and /sys stand-ins from state/proc.json."""
    proc = json.loads((ctl / "state/proc.json").read_text())
    fake = ctl / "fake"
    (fake / "proc/sys/kernel/random").mkdir(parents=True, exist_ok=True)
    (fake / "proc/pressure").mkdir(parents=True, exist_ok=True)
    (fake / "sys/class/thermal/thermal_zone0").mkdir(parents=True, exist_ok=True)
    (fake / "proc/meminfo").write_text(meminfo_lines(proc["meminfo"]))
    (fake / "proc/cpuinfo").write_text("processor : 0\nCPU architecture: 8\n")
    (fake / "proc/loadavg").write_text(proc["loadavg"] + "\n")
    (fake / "proc/vmstat").write_text("".join(f"{k} {v}\n" for k, v in proc["vmstat"].items()))
    (fake / "proc/pressure/memory").write_text("some avg10=0.00 avg60=0.00 avg300=0.00 total=0\n")
    (fake / "proc/sys/kernel/random/boot_id").write_text(proc["boot_id"] + "\n")
    (fake / "sys/class/thermal/thermal_zone0/temp").write_text(f"{proc['temperature']}\n")


def change_proc(ctl, **changes):
    lock = locked("proc")
    try:
        proc = json.loads((ctl / "state/proc.json").read_text())
        for key, value in changes.items():
            if key in proc["meminfo"]:
                proc["meminfo"][key] = value
            elif key in proc["vmstat"]:
                proc["vmstat"][key] = value
            else:
                proc[key] = value
        (ctl / "state/proc.json").write_text(json.dumps(proc, indent=1))
        render_proc(ctl)
    finally:
        lock.close()


def stay(seconds):
    """Sleep inside the guest; the guest's processes die with it, so leave when QEMU is gone."""
    port = os.environ.get("LIMEOS_REHEARSAL_GUEST_PORT")
    end = time.time() + seconds
    while time.time() < end:
        try:
            os.kill(int((CTL / "guests" / port / "qemu.pid").read_text()), 0)
        except (OSError, ValueError, TypeError):
            sys.exit(255)
        time.sleep(0.2)


def guest_suite(argv):
    script = Path(argv[0]).name
    suite = {"install_guest.py": "install", "approved_guest.py": "container"}.get(script)
    suite = suite or ("scale" if "scale" in argv else "stack")
    output = Path(argv[argv.index("--output") + 1])
    config = scenario().get("suites", {}).get(suite, {})
    (CTL / "markers").mkdir(exist_ok=True)
    (CTL / "markers" / f"{suite}.started").write_text(str(time.time()))
    began = time.time()
    for action in sorted(config.get("actions", []), key=lambda a: a["at"]):
        stay(max(0, began + action["at"] - time.time()))
        if action["do"] == "docker_stop":
            stop_container(action["name"])
        elif action["do"] == "meminfo":
            change_proc(CTL, **action["set"])
        elif action["do"] == "units":
            state("units.txt").write_text(action["text"])
    stay(float(config.get("sleep", 0)))
    if config.get("write_result", True):
        output.parent.mkdir(parents=True, exist_ok=True)
        body = json.dumps({"summary": config.get("summary", "pass"), "suite": suite, "stub": True})
        output.write_text(body + "\n")
        if suite != "install":
            output.with_name(output.stem + "-fixture.json").write_text(body + "\n")
    print(f"rehearsal guest suite {suite}: {config.get('summary', 'pass')}")
    return int(config.get("exit", 0))


def guest_python3(argv):
    if argv and argv[0].endswith("_guest.py"):
        return guest_suite(argv)
    os.execv(sys.executable, [sys.executable, *argv])


def guest_journalctl(argv):
    print("-- No entries --")
    return 0


def guest_systemctl(argv):
    print("0 loaded units listed.")
    return 0


STUBS = {
    "docker": docker,
    "dpkg-query": dpkg_query,
    "apt-mark": apt_mark,
    "apt-get": apt_get,
    "uname": uname,
    "free": free,
    "getconf": getconf,
    "systemctl": systemctl,
    "ss": ss,
    "journalctl": journalctl,
    "sudo": sudo,
    "install": install,
    "qemu-img": qemu_img,
    "qemu-system-aarch64": qemu,
    "guest-python3": guest_python3,
    "guest-journalctl": guest_journalctl,
    "guest-systemctl": guest_systemctl,
}


def main(name, ctl, argv=None):
    global CTL
    CTL = Path(ctl)
    sys.path.insert(0, str(HERE))
    argv = sys.argv[1:] if argv is None else argv
    log_call(name, argv)
    return STUBS[name](argv)


if __name__ == "__main__":
    # `remote_stubs.py NAME CTL args...`: used by the qemu launcher (exec -a keeps argv[0]).
    sys.exit(main(sys.argv[1], sys.argv[2], sys.argv[3:]))
