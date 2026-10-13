import hashlib
import json
import pathlib
import subprocess
import time
base = pathlib.Path.home() / 'limeos-arm64-qual/image'
expected = '835b929522716de9798d1c0fbe61caae8313ab78858b278f3389a2db922ae42e4619b4a776ecad39583c149fb85334a2f8cdae685bc56987160e5c86a2354375'
lines = (base / 'SHA512SUMS').read_text().splitlines()
matching = [line for line in lines if line.endswith('  debian-12-genericcloud-arm64.qcow2')]
assert len(matching) == 1 and matching[0].split()[0] == expected
with (base / 'debian-12-genericcloud-arm64.qcow2').open('rb') as stream:
    actual = hashlib.file_digest(stream, 'sha512').hexdigest()
assert actual == expected, actual
pid = int(subprocess.check_output(['systemctl', 'show', 'pi-health', '--property=MainPID', '--value'], text=True))
stat = pathlib.Path(f'/proc/{pid}/stat').read_text()
start = int(stat[stat.rindex(')')+2:].split()[19])
assert subprocess.check_output(['systemctl', 'is-active', 'pi-health', 'pihealth-helper', 'docker'], text=True).splitlines() == ['active']*3
print(json.dumps({'captured_epoch': time.time(), 'image_sha512': actual, 'checksum_manifest_sha256': hashlib.sha256((base/'SHA512SUMS').read_bytes()).hexdigest(), 'pi_health_identity': {'pid':pid,'start_ticks':start}, 'no_existing_qemu': subprocess.run(['pgrep','-a','qemu'], capture_output=True, text=True).stdout == ''}, indent=2))
