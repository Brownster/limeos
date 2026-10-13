"""Install only the operator-reviewed missing VM prerequisites, with exact deltas."""
import hashlib
import json
import pathlib
import re
import subprocess

EXPECTED = set('''libaio1 libbrlapi0.8 libcacard0 libcapstone4 libdaxctl1 libdrm-amdgpu1 libdrm-radeon1 libepoxy0 libexecs0 libwayland-server0 libxcb-randr0 libglapi-mesa libz3-4 libllvm15 libsensors-config libsensors5 libx11-xcb1 libxcb-dri2-0 libxcb-dri3-0 libxcb-present0 libxcb-sync1 libxcb-xfixes0 libxshmfence1 mesa-libgallium libgbm1 libgstreamer1.0-0 liborc-0.4-0 libgstreamer-plugins-base1.0-0 libndctl6 libopus0 libpixman-1-0 libpmem1 libsndio7.0 libspice-server1 libusbredirparser1 libvdeplug2 qemu-efi-aarch64 qemu-system-common qemu-system-data qemu-system-arm qemu-utils'''.split())
BASE = pathlib.Path.home() / 'limeos-arm64-qual'
assert not BASE.exists(), 'owned staging directory unexpectedly exists'
assert subprocess.check_output(['uname', '-m'], text=True).strip() == 'aarch64'
assert pathlib.Path('/dev/kvm').is_char_device()
subprocess.run(['sudo', '-n', 'true'], check=True)

def packages():
    text = subprocess.check_output(['dpkg-query', '-W', '-f=${binary:Package}\t${Version}\t${db:Status-Abbrev}\n'], text=True)
    return text, {line.split('\t')[0]: line for line in text.splitlines()}

before, rows = packages()
manual_before = subprocess.check_output(['apt-mark', 'showmanual'], text=True)
simulation = subprocess.check_output(['sudo', '-n', 'apt-get', '-s', '--no-install-recommends', '--no-upgrade', 'install', 'qemu-system-arm', 'qemu-efi-aarch64', 'qemu-utils'], text=True)
planned = set(re.findall(r'^Inst ([^ ]+) ', simulation, re.MULTILINE))
assert planned == EXPECTED, ('package set changed', sorted(planned ^ EXPECTED))
assert not re.search(r'^Remv ', simulation, re.MULTILINE), 'package removal proposed'
assert '0 upgraded, 41 newly installed, 0 to remove' in simulation, 'existing upgrades or unexpected count'
for name in planned:
    matching = [line for key, line in rows.items() if key.split(':')[0] == name]
    assert not matching, ('baseline already contains a proposed package', name, matching)
BASE.mkdir(mode=0o700)
(BASE / 'preparation-before.json').write_text(json.dumps({'packages': before, 'manual_packages': manual_before, 'simulation': simulation, 'expected_added': sorted(EXPECTED)}, indent=2)+'\n')
with (BASE / 'preparation-install.txt').open('w') as stream:
    subprocess.run(['sudo', '-n', 'env', 'DEBIAN_FRONTEND=noninteractive', 'apt-get', 'install', '-y', '--no-install-recommends', '--no-upgrade', 'qemu-system-arm', 'qemu-efi-aarch64', 'qemu-utils'], stdout=stream, stderr=subprocess.STDOUT, check=True, timeout=600)
after, new_rows = packages()
changed = [key for key, value in rows.items() if new_rows.get(key) != value]
added = {key.split(':')[0] for key in set(new_rows)-set(rows)}
result = {'packages': after, 'manual_packages': subprocess.check_output(['apt-mark', 'showmanual'], text=True), 'added': sorted(added), 'changed_existing': changed, 'before_sha256': hashlib.sha256(before.encode()).hexdigest(), 'after_sha256': hashlib.sha256(after.encode()).hexdigest()}
(BASE / 'preparation-after.json').write_text(json.dumps(result, indent=2)+'\n')
assert not changed and added == EXPECTED, result
print(json.dumps({key: value for key, value in result.items() if key not in ('packages', 'manual_packages')}, indent=2))
print(subprocess.check_output(['qemu-system-aarch64', '--version'], text=True).splitlines()[0])
