# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""Preserve public signing metadata and genuine previous-artifact checks."""
import datetime
import gzip
import hashlib
import json
import pathlib
import shutil
import subprocess

from ci_provenance import file_map, package_payload, verify_repository
from qualification import load_build, sha
from upgrade_guest import validate_previous, verify_previous_package

root = pathlib.Path('/home/marc/Documents/github/lime-os')
base = root / '.cache/arm64-qual/integrated-2026-10-12'
ready = base / 'ci-ready-2026-10-13'
public = ready / 'public-handoff'
public.mkdir()
build = load_build(ready / 'prepared/build-result.json', ready / 'repo')
previous_path = base / 'upgrade-inputs/schema8/previous-artifact.json'
previous = json.loads(previous_path.read_text())
validate_previous(previous, build['identity'])
package = verify_previous_package(ready / 'previous-repo', previous)
observed, _ = package_payload(package)
assert observed['binaries']['limeos-core']['sha256'] == previous['core_sha256']
assert subprocess.run(['dpkg', '--compare-versions', build['identity']['package_version'], 'gt', previous['package_version']], check=False).returncode == 0
for name in ('repo', 'previous-repo'):
    files = verify_repository(ready / name, build['signing_fingerprint'])
    target = public / name
    for relative in files:
        if relative.endswith('.deb'):
            continue
        destination = target / relative
        destination.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(ready / name / relative, destination)
previous_record = {'original_provenance_sha256': sha(previous_path), 'previous': previous, 'repository_sha256': file_map(ready / 'previous-repo'), 'signing_fingerprint': build['signing_fingerprint'], 'package_unchanged': True, 'core_unchanged': True, 'new_version_is_greater': True, 'actual_upgrade_test': 'untested'}
(public / 'previous-repository.json').write_text(json.dumps(previous_record, indent=2) + '\n')
shutil.copy2(previous_path, public / 'previous-artifact.json')
shutil.copy2(ready / 'signing-public.json', public / 'signing-public.json')
signing = (ready / 'signing.log').read_bytes()
(public / 'signing.log.gz').write_bytes(gzip.compress(signing, mtime=0))
image = json.loads(subprocess.check_output(['docker', 'image', 'inspect', 'limeos-arm64-repo-tools:local']))[0]
(public / 'repository-tool-image.json').write_text(json.dumps({'id': image['Id'], 'platform': image['Os'] + '/' + image['Architecture'], 'purpose': 'local apt-ftparchive/GPG tooling; not the native CI release builder'}, indent=2) + '\n')
for name in ('Containerfile', 'sign_repositories.py', 'capture_validation.py', 'capture_validation_broad.py', 'prepare_handoff.py'):
    shutil.copy2(root / '.cache/arm64-ci-input-validation' / name, public / name)
broader = ready / 'local-validation'
probe = {'fixture_commit': build['identity']['fixtures_commit'], 'debian_python311_harness': {'exit': 0, 'tests': 61, 'log': 'debian-python311-harness.log.gz'}, 'whole_directory_ruff_probe': {'exit': 1, 'log': 'ruff-check.log.gz', 'finding': 'Two pre-existing UP017 timezone.utc aliases in untouched native_vm.py; changed-file Ruff gates pass separately'}, 'logs': {p.name: {'compressed_sha256': sha(p), 'raw_sha256': hashlib.sha256(gzip.decompress(p.read_bytes())).hexdigest()} for p in sorted(broader.glob('*.gz'))}}
(broader / 'observations.json').write_text(json.dumps(probe, indent=2) + '\n')
handoff = {'recorded_at': datetime.datetime.now(datetime.UTC).isoformat(), 'identity': build['identity'], 'bundle_sha256': sha(ready / 'bundle-final/source.tar.gz'), 'source_manifest_sha256': sha(ready / 'bundle-final/source-manifest.json'), 'build_result_sha256': sha(ready / 'prepared/build-result.json'), 'source_files': len(build['runtime_source_sha256']), 'fixture_files': len(build['fixtures_sha256']), 'frontend_files': len(build['frontend_dist_sha256']), 'ready_directory': str(ready), 'guest_copy': {'expected': 'prepared/build-result.json', 'required_sidecar': 'prepared/inputs/', 'current_repo': 'prepared/inputs/repo/', 'previous_repo': 'previous-repo/', 'previous_provenance': 'public-handoff/previous-artifact.json'}, 'signing_fingerprint': build['signing_fingerprint'], 'key_lifetime': '2d', 'package_sha256': build['packages'], 'native_ci_input_archive_sha256': build['ci']['observations']['artifact']['digest'], 'local_tests': 61, 'native_guest_or_pi_tests_performed': False, 'pi_accessed': False, 'untested': ['install', 'footprint', 'upgrade', 'storage crash recovery', 'container crash recovery', 'bare-metal 16KiB pages', 'read-only production shadow', 'migration and return', 'P04 live storage/restore'], 'signing_log_raw_sha256': hashlib.sha256(signing).hexdigest()}
(public / 'handoff.json').write_text(json.dumps(handoff, indent=2) + '\n')
print(json.dumps({'public_handoff': str(public), 'previous_package_unchanged': True, 'source_files': handoff['source_files'], 'fixture_files': handoff['fixture_files']}))
