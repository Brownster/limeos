"""Light passive host health evidence while a disposable guest is active."""
import json
import pathlib
import subprocess
import time
import urllib.request

def cmd(*argv):
    result = subprocess.run(argv, capture_output=True, text=True, timeout=15)
    return {"exit": result.returncode, "stdout": result.stdout, "stderr": result.stderr}

result = {"captured_utc": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
          "meminfo": pathlib.Path("/proc/meminfo").read_text(),
          "vmstat": {line.split()[0]: int(line.split()[1]) for line in pathlib.Path("/proc/vmstat").read_text().splitlines() if line.startswith(("pswpin ", "pswpout ", "oom_kill "))},
          "loadavg": pathlib.Path("/proc/loadavg").read_text().strip(),
          "temperature_millidegrees": int(pathlib.Path("/sys/class/thermal/thermal_zone0/temp").read_text()),
          "boot_id": pathlib.Path("/proc/sys/kernel/random/boot_id").read_text().strip()}
result["services"] = cmd("systemctl", "show", "pi-health", "pihealth-helper", "docker", "smbd", "--property=Id,ActiveState,SubState,MainPID,NRestarts")
result["containers"] = cmd("docker", "ps", "-a", "--no-trunc", "--format", "{{.ID}}\t{{.Names}}\t{{.State}}\t{{.Status}}")
result["test_processes"] = []
for directory in pathlib.Path('/proc').iterdir():
    if not directory.name.isdigit():
        continue
    try:
        argv = (directory / 'cmdline').read_bytes().split(b'\0')
        argv = [part.decode(errors='replace') for part in argv if part]
        if not argv or not (pathlib.Path(argv[0]).name == 'qemu-system-aarch64' or any(pathlib.Path(part).name == 'guest_supervisor.py' for part in argv[:4])):
            continue
        stat = (directory / 'stat').read_text()
        result['test_processes'].append({'pid':int(directory.name), 'state':stat[stat.rindex(')')+2:].split()[0], 'start_ticks':int(stat[stat.rindex(')')+2:].split()[19]), 'argv':argv})
    except (OSError, ValueError, IndexError):
        pass
result["http"] = {}
for url in ["http://127.0.0.1:8002/", "http://127.0.0.1:8096/health"]:
    try:
        started = time.monotonic()
        with urllib.request.urlopen(url, timeout=5) as response:
            response.read(32768)
            result["http"][url] = {"status": response.status, "seconds": round(time.monotonic()-started, 4)}
    except OSError as error:
        result["http"][url] = {"error": str(error)}
print(json.dumps(result, indent=2))
