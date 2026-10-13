"""Read-only Pi baseline. No config contents, credentials or media are collected."""
import hashlib
import json
import pathlib
import subprocess
import time
import urllib.request
import urllib.error

def cmd(*argv):
    result = subprocess.run(argv, capture_output=True, text=True, timeout=30)
    return {"exit": result.returncode, "stdout": result.stdout, "stderr": result.stderr}

result = {"captured_utc": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime())}
for name, argv in {
    "identity": ["uname", "-a"],
    "uptime": ["uptime"],
    "memory": ["free", "-k"],
    "filesystem": ["df", "-k", "/home/holly", "/tmp"],
    "mounts": ["findmnt", "--json", "--output", "TARGET,SOURCE,FSTYPE,OPTIONS"],
    "failed_units": ["systemctl", "--failed", "--no-pager", "--plain"],
    "services": ["systemctl", "show", "pi-health.service", "pihealth-helper.service", "docker.service", "smbd.service", "nmbd.service", "limeos-agent.service", "--property=Id,ActiveState,SubState,MainPID,NRestarts,ExecMainStartTimestamp,FragmentPath"],
    "containers": ["docker", "ps", "-a", "--no-trunc", "--format", "{{.ID}}\t{{.Names}}\t{{.State}}\t{{.Status}}"],
    "container_health": ["docker", "inspect", "--format", "{{.Id}} {{.Name}} {{.State.Status}} {{.State.StartedAt}} {{.State.FinishedAt}} {{.RestartCount}} {{if .State.Health}}{{.State.Health.Status}}{{end}}", *subprocess.check_output(["docker", "ps", "-aq"], text=True).split()],
    "packages": ["dpkg-query", "-W", "-f=${binary:Package}\t${Version}\t${db:Status-Abbrev}\n"],
    "sudo": ["sudo", "-n", "true"],
    "temperature": ["cat", "/sys/class/thermal/thermal_zone0/temp"],
    "throttled": ["vcgencmd", "get_throttled"],
    "test_processes": ["pgrep", "-af", "qemu-system|guest_supervisor"],
    "kernel_events": ["sudo", "-n", "journalctl", "-k", "--since", "2026-10-12 21:44:28 UTC", "--no-pager", "--grep", "Out of memory|oom-kill|Killed process|I/O error|EXT4-fs error|nvme.*error|under-voltage|throttl"],
    "package_simulation": ["sudo", "-n", "apt-get", "-s", "--no-install-recommends", "install", "qemu-system-arm", "qemu-efi-aarch64", "qemu-utils"],
}.items():
    try:
        result[name] = cmd(*argv)
    except (OSError, subprocess.TimeoutExpired) as error:
        result[name] = {"unavailable": str(error)}
result["boot_id"] = pathlib.Path("/proc/sys/kernel/random/boot_id").read_text().strip()
result["meminfo"] = pathlib.Path("/proc/meminfo").read_text()
result["vmstat"] = pathlib.Path("/proc/vmstat").read_text()
result["configuration_hashes"] = {}
for path in ["/etc/fstab", "/etc/snapraid.conf", "/etc/samba/smb.conf"]:
    try:
        result["configuration_hashes"][path] = hashlib.sha256(pathlib.Path(path).read_bytes()).hexdigest()
    except OSError as error:
        result["configuration_hashes"][path] = {"unavailable": str(error)}
result["http"] = {}
for url in ["http://127.0.0.1:8002/", "http://127.0.0.1:8096/health"]:
    try:
        with urllib.request.urlopen(url, timeout=10) as response:
            result["http"][url] = {"status": response.status, "final_url": response.url, "bytes_read": len(response.read(32768))}
    except (OSError, urllib.error.URLError) as error:
        result["http"][url] = {"error": str(error)}
print(json.dumps(result, indent=2))
