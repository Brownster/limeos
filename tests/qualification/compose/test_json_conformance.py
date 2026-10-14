# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""Opt-in offline conformance of shared Rust fixtures against a pinned parser.

Run unittest discovery with LIMEOS_COMPOSE_CONFORMANCE_BIN explicitly set.
Only config/version commands run; daemon and ambient environment are excluded.
"""
import hashlib
import json
import os
import platform
from pathlib import Path
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[3]
FIXTURE = ROOT / 'tests/fixtures/compose-source'
PIN = json.loads((FIXTURE / 'conformance-pin.json').read_text())
CASES = {case['id']: case for case in json.loads((FIXTURE / 'conformance-cases.json').read_text())['cases']}
PARSER = os.environ.get('LIMEOS_COMPOSE_CONFORMANCE_BIN')


def sha256(path):
    digest = hashlib.sha256()
    with path.open('rb') as stream:
        for chunk in iter(lambda: stream.read(64 * 1024), b''):
            digest.update(chunk)
    return digest.hexdigest()


def projected_services(model):
    services = []
    for name, value in model['services'].items():
        assert set(value) <= {'image', 'user', 'restart', 'privileged', 'network_mode',
                              'networks', 'ports', 'volumes', 'devices'}, value
        if value.get('network_mode') == 'host':
            assert 'networks' not in value
        else:
            assert value.get('networks') == {'default': None}
        user = value.get('user')
        if user is not None:
            uid, gid = user.split(':')
            user = dict(uid=int(uid), gid=int(gid))
        ports = []
        for port in value.get('ports', []):
            assert set(port) <= {'host_ip', 'published', 'target', 'protocol', 'mode'}
            assert port.get('mode', 'ingress') == 'ingress'
            ports.append(dict(host_ip=port['host_ip'], published=int(port['published']),
                              target=port['target'], protocol=port['protocol']))
        mounts = []
        for mount in value.get('volumes', []):
            assert set(mount) <= {'type', 'source', 'target', 'read_only', 'bind'}
            result = dict(kind=mount['type'], target=mount['target'], read_only=mount.get('read_only', False))
            if mount['type'] == 'bind':
                assert mount['bind'] == {'create_host_path': False}
                result['source'] = mount['source']
            else:
                assert mount['type'] == 'volume' and 'bind' not in mount
                result['name'] = mount['source']
            mounts.append(result)
        devices = []
        for device in value.get('devices', []):
            assert set(device) == {'source', 'target', 'permissions'} and device['permissions'] == 'rwm'
            devices.append(dict(source=device['source'], target=device['target']))
        services.append(dict(name=name, image=value['image'], user=user, restart=value.get('restart', 'no'),
            privileged=value.get('privileged', False), host_network=value.get('network_mode') == 'host',
            ports=ports, mounts=mounts, devices=devices))
    return {'services': services}


def ordered(value):
    if isinstance(value, list):
        return sorted((ordered(item) for item in value), key=lambda item: json.dumps(item, sort_keys=True))
    if isinstance(value, dict):
        return {key: ordered(item) for key, item in value.items()}
    return value


@unittest.skipUnless(PARSER, 'explicit pinned Compose parser opt-in required')
class JsonConformance(unittest.TestCase):
    def run_case(self, name):
        case = CASES[name]
        parser = Path(PARSER).resolve(strict=True)
        self.assertEqual(platform.machine(), 'x86_64')
        self.assertEqual(sha256(parser), PIN['parser_sha256'])
        scratch_base = Path(os.environ.get('TMPDIR', ROOT / '.cache/tmp'))
        with tempfile.TemporaryDirectory(prefix='limeos-compose-config-', dir=scratch_base) as temporary:
            scratch = Path(temporary)
            config = scratch / 'docker-config'
            config.mkdir()
            endpoint = scratch / 'no-daemon.sock'
            environment = {'PATH': '/usr/bin:/bin', 'DOCKER_HOST': 'unix://' + str(endpoint),
                           'DOCKER_CONFIG': str(config), 'COMPOSE_DISABLE_ENV_FILE': '1'}
            # An invalid ambient trap proves default .env loading is disabled.
            (scratch / '.env').write_text('invalid variable name = unterminated"\n')
            operator = scratch / 'compose.yaml'
            managed = scratch / 'limeos.override.json'
            operator.write_text(json.dumps(case['operator']))
            managed.write_text(json.dumps(case['artifact']))
            command = [str(parser), '--project-name', 'limeos-conformance', '--project-directory', str(scratch),
                       '--file', str(operator), '--file', str(managed), 'config', '--format', 'json',
                       '--no-env-resolution', '--no-interpolate', '--no-path-resolution']
            version = subprocess.run([str(parser), 'version', '--short'], cwd=scratch, env=environment,
                                     capture_output=True, check=False, timeout=5)
            self.assertEqual(version.returncode, 0, version.stderr)
            self.assertEqual(version.stdout.decode().strip(), PIN['parser_version'])
            result = subprocess.run(command, cwd=scratch, env=environment, capture_output=True,
                                    check=False, timeout=5)
            print(json.dumps({'case': name, 'command': command, 'environment': environment,
                'architecture': platform.machine(), 'parser_version': version.stdout.decode().strip(), 'parser_sha256': sha256(parser),
                'version_command': [str(parser), 'version', '--short'], 'version_exit': version.returncode,
                'version_stdout': version.stdout.decode(), 'version_stderr': version.stderr.decode(),
                'exit': result.returncode, 'stdout': result.stdout.decode(), 'stderr': result.stderr.decode()}), flush=True)
            self.assertEqual(sha256(parser), PIN['parser_sha256'])
            self.assertEqual(result.returncode, 0, result.stderr)
            model = json.loads(result.stdout)
            self.assertEqual(set(model) - {'name', 'services', 'networks', 'volumes'}, set())
            self.assertEqual(model['name'], 'limeos-conformance')
            for volume, definition in model.get('volumes', {}).items():
                self.assertEqual(definition, {'name': 'limeos-conformance_' + volume})
            self.assertEqual(ordered(projected_services(model)), ordered(case['desired']))
            self.assertFalse(endpoint.exists())
            self.assertEqual(list(config.iterdir()), [])
            self.assertEqual(json.loads(operator.read_text()), case['operator'])
            self.assertEqual(json.loads(managed.read_text()), case['artifact'])

    def test_new_service(self):
        self.run_case('new_service')

    def test_merge_delta(self):
        self.run_case('merge_delta')

    def test_scalar_false(self):
        self.run_case('scalar_false')

    def test_remove_one_managed(self):
        self.run_case('remove_one_managed')

    def test_host_mode(self):
        self.run_case('host_mode')

    def test_scalar_update_keeps_sequences(self):
        self.run_case('scalar_update_keeps_sequences')
