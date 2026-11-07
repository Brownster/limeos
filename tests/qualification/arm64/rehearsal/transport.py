#!/usr/bin/env python3
"""Local stand-ins for `ssh` and `scp`, used only by the Pi night rehearsal.

They keep the ssh semantics that broke real runs: the remote command is ONE string
run by a shell (`bash -c`); the call returns only when the command has exited AND
every holder of its stdout/stderr pipes has closed them (so a backgrounded job that
inherited the pipes hangs the call, exactly as with ssh); stdin is forwarded; the exit
status is the shell's; and a caller that kills the client on a timeout leaves the
remote process behind, as sshd does without a tty.

Nothing here opens a network connection. A call is served only for the one rehearsal
host named in LIMEOS_REHEARSAL_HOST (and the loopback guests behind it); any other
host name is refused with ssh's own "could not resolve" failure, status 255.

Fault injection comes from `<ctl>/faults.json`, a list of
{"match": regex, "action": "hang"|"fail"|"hold"|"delay", "times": n, "seconds": s,
 "target": "host"|"guest"|"scp"}; it is how the rehearsal proves per-call timeouts.
"""

import fcntl
import json
import os
import re
import subprocess
import sys
import threading
import time
from pathlib import Path

HOST_ENV = "LIMEOS_REHEARSAL_HOST"
HOME_ENV = "LIMEOS_REHEARSAL_HOME"
CTL_ENV = "LIMEOS_REHEARSAL_CTL"
TOKEN_ENV = "LIMEOS_REHEARSAL_TOKEN"
VALUE_OPTIONS = set("bcDEeFIiJLlmOopQRSWw")


class Refusal(Exception):
    pass


def settings():
    host, home, ctl = (os.environ.get(k) for k in (HOST_ENV, HOME_ENV, CTL_ENV))
    if not (host and home and ctl):
        raise Refusal("rehearsal ssh shim without LIMEOS_REHEARSAL_* settings")
    if host.rsplit("@", 1)[-1].casefold() in ("wybie", "holly"):
        raise Refusal("the rehearsal shim never serves a production host name")
    return host, Path(home), Path(ctl)


def parse_ssh(argv):
    """Return (options dict of -o values, flags, positional list, value options)."""
    options, values, flags, i = [], {}, set(), 0
    while i < len(argv):
        arg = argv[i]
        if arg == "--":
            i += 1
            break
        if not arg.startswith("-") or arg == "-":
            break
        letter = arg[1]
        if letter == "o":
            if len(arg) > 2:
                options.append(arg[2:])
            else:
                i += 1
                options.append(argv[i])
        elif letter in VALUE_OPTIONS:
            if len(arg) > 2:
                values[letter] = arg[2:]
            else:
                i += 1
                values[letter] = argv[i]
        else:
            flags.update(arg[1:])
        i += 1
    opts = {}
    for item in options:
        key, _, value = item.partition("=")
        opts[key.strip()] = value.strip()
    return opts, values, flags, argv[i:]


# ----------------------------------------------------------------- path mapping
HOST_MAP = (
    (r"/usr/share/AAVMF/", "{ctl}/usr/share/AAVMF/"),
    (r"/dev/kvm\b", "/dev/null"),
    (r"/proc/(meminfo|cpuinfo|loadavg|vmstat|pressure/memory)\b", "{ctl}/fake/proc/\\1"),
)
GUEST_MAP = ((r"/(root|opt|var)/", "{guest}/\\1/"),)


def map_paths(text, table, **names):
    for pattern, replacement in table:
        text = re.sub(
            r"(?<![\w.\-/])" + pattern, replacement.format(**names), text
        )
    return text


# ------------------------------------------------------------------- faults
def fault_for(ctl, target, command):
    path = ctl / "faults.json"
    if not path.exists():
        return None
    with (ctl / "faults.lock").open("a+") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX)
        faults = json.loads(path.read_text())
        for fault in faults:
            if fault.get("target", "host") != target:
                continue
            if not re.search(fault["match"], command):
                continue
            if fault.get("times") is not None:
                if fault["times"] <= 0:
                    continue
                fault["times"] -= 1
                path.write_text(json.dumps(faults))
            return fault
    return None


def record(ctl, kind, **fields):
    line = json.dumps({"epoch": round(time.time(), 3), "kind": kind, **fields}) + "\n"
    with (ctl / "transport.jsonl").open("a") as stream:
        stream.write(line)


def hang_forever():
    while True:
        time.sleep(3600)


# ---------------------------------------------------------------------- run
def run_session(argv, env, cwd):
    """ssh session: wait for exit AND EOF on both pipes, forward stdin, relay streams."""
    process = subprocess.Popen(
        argv,
        env=env,
        cwd=cwd,
        stdin=0,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        start_new_session=True,
    )

    def pump(source, sink):
        while chunk := os.read(source.fileno(), 65536):
            os.write(sink, chunk)

    threads = [
        threading.Thread(target=pump, args=(process.stdout, 1), daemon=True),
        threading.Thread(target=pump, args=(process.stderr, 2), daemon=True),
    ]
    for thread in threads:
        thread.start()
    status = process.wait()
    for thread in threads:
        thread.join()
    return status


def remote_env(host_home, ctl, extra=None):
    env = {
        "HOME": str(host_home),
        "USER": "holly",
        "LOGNAME": "holly",
        "SHELL": "/bin/bash",
        "LANG": "C.UTF-8",
        "PATH": f"{ctl}/bin:/usr/local/bin:/usr/bin:/bin",
        "PWD": str(host_home),
        "LIMEOS_QUAL_PROC_ROOT": f"{ctl}/fake",
        CTL_ENV: str(ctl),
        HOME_ENV: str(host_home),
        TOKEN_ENV: os.environ.get(TOKEN_ENV, ""),
    }
    env.update(extra or {})
    return env


def ssh_main(argv):
    try:
        host, home, ctl = settings()
    except Refusal as error:
        print(f"ssh: {error}", file=sys.stderr)
        return 255
    opts, values, flags, positional = parse_ssh(argv)
    if "G" in flags:
        target = positional[0] if positional else ""
        print(f"hostname {target.rsplit('@', 1)[-1]}")
        return 0
    if not positional:
        print("usage: ssh [options] host [command]", file=sys.stderr)
        return 255
    target, command = positional[0], " ".join(positional[1:])
    guest_port = None
    if target == host:
        kind = "host"
    elif target == "root@127.0.0.1" and opts.get("ProxyJump") == host:
        kind = "guest"
        guest_port = values.get("p")
    else:
        print(f"ssh: Could not resolve hostname {target}: Name or service not known", file=sys.stderr)
        return 255
    if opts.get("BatchMode") != "yes" and kind == "host":
        print("ssh: BatchMode=yes is required for unattended runs", file=sys.stderr)
        return 255
    record(ctl, "ssh", target=kind, port=guest_port, command=command)
    fault = fault_for(ctl, kind, command)
    if fault:
        action = fault["action"]
        if action == "hang":
            hang_forever()
        if action == "fail":
            print("ssh: connect to host port 22: Connection timed out", file=sys.stderr)
            return 255
        if action == "delay":
            time.sleep(float(fault.get("seconds", 1)))
        if action == "hold":
            # The command runs, then a leftover job keeps the session's pipes open.
            command = f"{{ {command}\n}} ; (sleep 3600 &) ; true"
    if not command:
        print("ssh: interactive sessions are not available in the rehearsal", file=sys.stderr)
        return 255
    if kind == "host":
        return run_session(
            ["/bin/bash", "-c", map_paths(command, HOST_MAP, ctl=ctl)],
            remote_env(home, ctl),
            home,
        )
    return guest_session(ctl, home, guest_port, command)


def guest_root(ctl, port):
    return ctl / "guests" / str(port)


def guest_alive(ctl, port):
    try:
        pid = int((guest_root(ctl, port) / "qemu.pid").read_text())
        os.kill(pid, 0)
        return True
    except (OSError, ValueError):
        return False


def guest_session(ctl, host_home, port, command):
    root = guest_root(ctl, port)
    if not guest_alive(ctl, port):
        print(f"ssh: connect to host 127.0.0.1 port {port}: Connection refused", file=sys.stderr)
        return 255
    env = {
        "HOME": f"{root}/root",
        "USER": "root",
        "PATH": f"{ctl}/guest-bin:/usr/bin:/bin",
        "LANG": "C.UTF-8",
        "LIMEOS_REHEARSAL_GUEST_PORT": str(port),
        CTL_ENV: str(ctl),
        TOKEN_ENV: os.environ.get(TOKEN_ENV, ""),
    }
    mapped = map_paths(command, GUEST_MAP, guest=root)
    process = subprocess.Popen(
        ["/bin/bash", "-c", mapped],
        env=env,
        cwd=f"{root}/root",
        stdin=0,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        start_new_session=True,
    )
    lost = threading.Event()

    def watch():
        while process.poll() is None:
            if not guest_alive(ctl, port):
                lost.set()
                try:
                    os.killpg(process.pid, 9)
                except OSError:
                    pass
                return
            time.sleep(0.2)

    def pump(source, sink):
        while chunk := os.read(source.fileno(), 65536):
            os.write(sink, chunk)

    threads = [
        threading.Thread(target=watch, daemon=True),
        threading.Thread(target=pump, args=(process.stdout, 1), daemon=True),
        threading.Thread(target=pump, args=(process.stderr, 2), daemon=True),
    ]
    for thread in threads:
        thread.start()
    status = process.wait()
    for thread in threads[1:]:
        thread.join()
    if lost.is_set():
        print("client_loop: send disconnect: Broken pipe", file=sys.stderr)
        return 255
    return status


# ---------------------------------------------------------------------- scp
def scp_main(argv):
    try:
        host, home, ctl = settings()
    except Refusal as error:
        print(f"scp: {error}", file=sys.stderr)
        return 1
    opts, values, flags, positional = parse_ssh(
        [a if a != "-P" else "-p" for a in argv]
    )
    if len(positional) != 2:
        print("scp: exactly one source and one destination are supported", file=sys.stderr)
        return 1
    source, destination = positional
    for end in (source, destination):
        remote = remote_part(end)
        if remote and remote[0] not in (host, "root@127.0.0.1"):
            print(f"ssh: Could not resolve hostname {remote[0]}", file=sys.stderr)
            return 255
    record(ctl, "scp", source=source, destination=destination)
    fault = fault_for(ctl, "scp", f"{source} {destination}")
    if fault:
        if fault["action"] == "hang":
            hang_forever()
        if fault["action"] == "fail":
            print("scp: Connection closed", file=sys.stderr)
            return 255
        if fault["action"] == "delay":
            time.sleep(float(fault.get("seconds", 1)))
    guest_port = values.get("p")

    def local(path, remote_spec):
        if remote_spec is None:
            return Path(path)
        who, where = remote_spec
        if who == "root@127.0.0.1":
            if opts.get("ProxyJump") != host or not guest_alive(ctl, guest_port):
                raise Refusal(
                    f"ssh: connect to host 127.0.0.1 port {guest_port}: Connection refused"
                )
            base = guest_root(ctl, guest_port)
            return Path(map_paths(where, GUEST_MAP, guest=base)) if where.startswith("/") else base / "root" / where
        if where.startswith("~/"):
            where = where[2:]
        return Path(where) if where.startswith("/") else home / where

    try:
        src_path = local(source, remote_part(source))
        dst_path = local(destination, remote_part(destination))
    except Refusal as error:
        print(str(error), file=sys.stderr)
        return 255
    if not src_path.exists():
        print(f"scp: {source}: No such file or directory", file=sys.stderr)
        return 1
    if src_path.is_dir():
        if "r" not in flags:
            print(f"scp: {source}: not a regular file", file=sys.stderr)
            return 1
        target = dst_path / src_path.name if dst_path.is_dir() else dst_path
        done = subprocess.run(["cp", "-a", "--", str(src_path), str(target)])
        return 1 if done.returncode else 0
    if dst_path.is_dir():
        dst_path = dst_path / src_path.name
    if not dst_path.parent.is_dir():
        print(f"scp: {destination}: No such file or directory", file=sys.stderr)
        return 1
    done = subprocess.run(["cp", "--", str(src_path), str(dst_path)])
    return 1 if done.returncode else 0


def remote_part(spec):
    """(user@host, path) for `host:path`; None for a local path."""
    if spec.startswith(("/", "./", "../")) or ":" not in spec:
        return None
    head, _, tail = spec.partition(":")
    if "/" in head:
        return None
    return head, tail


if __name__ == "__main__":
    sys.exit({"ssh": ssh_main, "scp": scp_main}[Path(sys.argv[0]).name](sys.argv[1:]))
