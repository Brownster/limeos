"""Local disposable Debian container; sign test inputs without altering packages."""
import hashlib
import json
import os
import pathlib
import subprocess

work = pathlib.Path('/work')
os.environ['GNUPGHOME'] = str(work / 'gnupg')
os.environ['TMPDIR'] = str(work / 'tmp')
(work / 'gnupg').mkdir(mode=0o700)
(work / 'tmp').mkdir()
with (work / 'signing.log').open('wb') as log:
    subprocess.run(['gpg', '--batch', '--pinentry-mode', 'loopback', '--passphrase', '', '--quick-generate-key', 'LimeOS disposable CI-input qualification <rewrite@test.invalid>', 'ed25519', 'sign', '2d'], check=True, stdout=log, stderr=subprocess.STDOUT)
    keys = subprocess.check_output(['gpg', '--batch', '--with-colons', '--list-secret-keys'], stderr=log, text=True)
    fingerprint = next(line.split(':')[9] for line in keys.splitlines() if line.startswith('fpr:'))
    current = sorted(pathlib.Path('/packages').glob('*.deb'))
    previous = pathlib.Path('/previous/limeos_0.4.7+arm64.1_arm64.deb')
    assert len(current) == 2
    assert hashlib.sha256(previous.read_bytes()).hexdigest() == '6c86199b253e155c94d96e1395c069c4188e11a2379f14eff97eacab0203b898'
    for name, packages in (('repo', current), ('previous-repo', [previous])):
        subprocess.run(['bash', '/packaging/repository.sh', str(work / name), fingerprint, *map(str, packages)], check=True, stdout=log, stderr=subprocess.STDOUT)
    versions = subprocess.check_output(['dpkg-query', '-W', '-f', '${Package} ${Version}\n', 'apt-utils', 'gnupg', 'dpkg'], text=True)
    public = {'signing_fingerprint': fingerprint, 'key_lifetime': '2d', 'previous_package_sha256': hashlib.sha256(previous.read_bytes()).hexdigest(), 'tool_versions': versions, 'production_key': False, 'pi_accessed': False}
    (work / 'signing-public.json').write_text(json.dumps(public, indent=2) + '\n')
    print(json.dumps(public))
