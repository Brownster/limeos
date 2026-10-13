# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""Sequential frozen-candidate guest tests with host health monitoring and teardown."""
import datetime
import hashlib
import json
import os
import pathlib
import shlex
import signal
import subprocess
import sys
import time

ROOT = pathlib.Path('/home/marc/Documents/github/lime-os')
RUN = ROOT / '.cache/arm64-qual/integrated-2026-10-12'
sys.path.insert(0, str(ROOT / 'tests/qualification/arm64'))
from native_vm import Guest
from qualification import load_build

IMAGE = '835b929522716de9798d1c0fbe61caae8313ab78858b278f3389a2db922ae42e4619b4a776ecad39583c149fb85334a2f8cdae685bc56987160e5c86a2354375'
DEADLINE = '2026-10-13T05:30:00Z'
EXPECTED_SOURCE = '016ba49a27b228576db5d4ca28dcd122c60a6278'
HEALTH = RUN / 'host-samples'
OUTPUT = RUN / 'sequence-logs'
HEALTH.mkdir(exist_ok=True)
OUTPUT.mkdir(exist_ok=True)
BASELINE = json.loads((RUN / 'host-baseline.json').read_text())
BASE_CONTAINERS = {line.split('\t')[0] for line in BASELINE['containers']['stdout'].splitlines() if line.split('\t')[2] == 'running'}
BASE_OOM = int(next(line.split()[1] for line in BASELINE['vmstat'].splitlines() if line.startswith('oom_kill ')))
sample_code = (RUN / 'host_sample.py').read_text()
status = {'pid':os.getpid(), 'started_utc':datetime.datetime.now(datetime.timezone.utc).isoformat(), 'source_commit':EXPECTED_SOURCE, 'steps':[], 'active_guest':None, 'summary':'incomplete'}
last_sample = 0
sample_number = 0

def save():
    temporary = RUN / 'sequence-status.tmp'
    temporary.write_text(json.dumps(status, indent=2)+'\n')
    temporary.replace(RUN / 'sequence-status.json')

def sample(enforce=True):
    global last_sample, sample_number
    last_sample = time.monotonic()
    sample_number += 1
    guest = Guest('health-sampler', 'holly@192.168.0.45', RUN / 'authorization-pi45.json')
    command = 'python3 -c ' + shlex.quote(sample_code)
    result = guest.host_run(command, check=False, capture_output=True, text=True, timeout=45)
    record = {'started':datetime.datetime.now(datetime.timezone.utc).isoformat(), 'host':guest.host, 'authorization_sha256':guest.authorization['sha256'], 'command':command, 'exit':result.returncode, 'stdout':result.stdout, 'stderr':result.stderr}
    with (RUN / 'host-commands-pi45.jsonl').open('a') as stream:
        stream.write(json.dumps(record)+'\n')
    path = HEALTH / f'{sample_number:03d}.json'
    if result.returncode:
        path.write_text(json.dumps(record, indent=2)+'\n')
        if enforce:
            raise RuntimeError('host health sample failed')
        return
    observed = json.loads(result.stdout)
    path.write_text(json.dumps(observed, indent=2)+'\n')
    memory = {line.split(':')[0]:int(line.split()[1]) for line in observed['meminfo'].splitlines()}
    running = {line.split('\t')[0] for line in observed['containers']['stdout'].splitlines() if line.split('\t')[2] == 'running'}
    failures = []
    if observed['boot_id'] != BASELINE['boot_id']:
        failures.append('host rebooted')
    if memory['MemAvailable'] < 2*1024*1024:
        failures.append('available host RAM below 2 GiB')
    if memory['SwapTotal'] - memory['SwapFree'] > 450*1024:
        failures.append('host swap exceeds 450 MiB')
    if observed['vmstat']['oom_kill'] != BASE_OOM:
        failures.append('new host OOM kill')
    if observed['temperature_millidegrees'] >= 75000:
        failures.append('host temperature at least 75 C')
    if BASE_CONTAINERS - running:
        failures.append('baseline production container stopped')
    if any(value.get('status') != 200 for value in observed['http'].values()):
        failures.append('production HTTP health failed')
    if observed['services']['stdout'].count('ActiveState=active') != 4:
        failures.append('production service inactive')
    status['last_host_sample'] = {'path':str(path.relative_to(RUN)), 'available_mib':round(memory['MemAvailable']/1024), 'swap_mib':round((memory['SwapTotal']-memory['SwapFree'])/1024), 'temperature_c':observed['temperature_millidegrees']/1000, 'http':observed['http'], 'failures':failures}
    save()
    print('HOST '+json.dumps(status['last_host_sample']), flush=True)
    if failures and enforce:
        raise RuntimeError('; '.join(failures))

def run(label, *arguments, required=True, monitor=True):
    arguments = tuple(map(str, arguments))
    status['current_step'] = label
    save()
    command = [sys.executable, str(ROOT / '.cache/pi-qualification.py'), '--label', label, '--', *map(str, arguments)]
    path = OUTPUT / (label + '.txt')
    started = time.monotonic()
    code = None
    with path.open('x') as output:
        process = subprocess.Popen(command, stdout=output, stderr=subprocess.STDOUT)
        try:
            while process.poll() is None:
                if monitor and time.monotonic()-last_sample >= 60:
                    sample()
                time.sleep(1)
            code = process.returncode
        finally:
            if process.poll() is None:
                process.terminate()
                try:
                    process.wait(timeout=10)
                except subprocess.TimeoutExpired:
                    process.kill()
                    process.wait()
            if code is None:
                code = process.returncode
            status['steps'].append({'label':label, 'argv':arguments, 'exit':code, 'seconds':round(time.monotonic()-started,3), 'log':str(path.relative_to(RUN))})
            save()
    print(f'STEP {label} exit={code}\n'+path.read_text()[-1600:], flush=True)
    if required and code:
        raise RuntimeError(f'{label} failed with exit {code}')
    return code

def boot(name, port, storage=False, build=False):
    status['active_guest'] = name
    save()
    extra = ['--storage-disks'] if storage else []
    run(name+'-boot', 'boot', name, '--port', str(port), '--cpus','2','--memory','1536','--disk','24G' if build else '16G','--image-sha512',IMAGE,'--timeout','450','--terminate-at',DEADLINE,*extra)

def stop(name):
    if (ROOT / '.cache/arm64-qual' / name / 'guest.json').exists():
        run(name+'-stop', 'stop',name, monitor=False)
    status['active_guest'] = None
    save()
    sample(enforce=False)

def source(name):
    run(name+'-mkdir','exec',name,'mkdir -p /root/qual')
    run(name+'-bundle','push',name,RUN/'source.tar.gz','/root/qual/source.tar.gz')
    run(name+'-manifest','push',name,RUN/'source-manifest.json','/root/qual/source-manifest.json')
    run(name+'-extract','exec',name,'cd /root/qual && tar -xzf source.tar.gz')

def installed_inputs(name):
    source(name)
    run(name+'-repo','push',name,RUN/'repo','/opt/limeos-repo')
    run(name+'-build-record','push',name,RUN/'build-result.json','/root/qual/build-result.json')
    run(name+'-hashes','exec',name,'cp /root/qual/source/tests/fixtures/werkzeug-hashes.json /root/werkzeug-hashes.json')

def collect(name, paths):
    directory = RUN / name
    directory.mkdir(exist_ok=True)
    for remote in paths:
        run(name+'-collect-'+pathlib.Path(remote).name.replace('.','-').replace('_','-'),'pull',name,remote,directory/pathlib.Path(remote).name,required=False)
    run(name+'-journals','exec',name,'journalctl -u "limeos-*" --no-pager > /root/qual/limeos-journal.txt; journalctl -k --no-pager > /root/qual/kernel-journal.txt; systemctl --failed --no-pager > /root/qual/failed-units.txt',required=False)
    for filename in ['limeos-journal.txt','kernel-journal.txt','failed-units.txt']:
        run(name+'-pull-'+filename.replace('.','-'),'pull',name,'/root/qual/'+filename,directory/filename,required=False)

def interrupted(signum, frame):
    raise KeyboardInterrupt(f'signal {signum}')

signal.signal(signal.SIGTERM, interrupted)
save()
try:
    sample()
    name = 'integrated-build-20261012'
    try:
        boot(name,22912,build=True)
        source(name)
        # Only this disposable build guest gets disk-backed emergency swap.
        setup = "import os,subprocess; subprocess.run(['fallocate','-l','512M','/root/qual/build-swap'],check=True); os.chmod('/root/qual/build-swap',0o600); subprocess.run(['mkswap','/root/qual/build-swap'],check=True); subprocess.run(['swapon','/root/qual/build-swap'],check=True)"
        run(name+'-build-swap','exec',name,'python3 -c '+shlex.quote(setup))
        run(name+'-native-build','exec',name,'RUST_TEST_THREADS=2 python3 /root/qual/fixtures/tests/qualification/arm64/build_guest.py --work /root/qual --manifest /root/qual/source-manifest.json',required=False)
        collect(name,['/root/qual/build-result.json','/root/qual/logs','/root/qual/packages','/root/qual/repo'])
        build_path = RUN / name / 'build-result.json'
        build = load_build(build_path)
        assert build['identity']['source_commit'] == EXPECTED_SOURCE
        import shutil
        shutil.copy2(build_path,RUN/'build-result.json')
        shutil.copytree(RUN/name/'repo',RUN/'repo')
        shutil.copytree(RUN/name/'packages',RUN/'packages')
        run(name+'-previous-mkdir','exec',name,'mkdir /root/qual/previous-input')
        previous = RUN / 'upgrade-inputs/schema8/limeos_0.4.7+arm64.1_arm64.deb'
        run(name+'-previous-artifact','push',name,previous,'/root/qual/previous-input/'+previous.name)
        key = build['signing_fingerprint']
        assert len(key)==40 and all(ch in '0123456789ABCDEFabcdef' for ch in key)
        run(name+'-previous-sign','exec',name,'cd /root/qual/source && bash packaging/repository.sh /root/qual/previous-repo '+shlex.quote(key)+' '+shlex.quote('/root/qual/previous-input/'+previous.name))
        run(name+'-previous-repo','pull',name,'/root/qual/previous-repo',RUN/'previous-repo')
    finally:
        stop(name)

    for suite, port in [('install',22913),('upgrade',22914),('storage',22915),('container',22916)]:
        name = 'integrated-'+suite+'-20261012'
        output = '/root/qual/'+suite+'-result.json'
        paths = [output]
        if suite in ('storage','container'):
            paths += ['/root/qual/'+suite+'-result-fixture.json']
        try:
            boot(name,port,storage=suite=='storage')
            installed_inputs(name)
            if suite == 'upgrade':
                run(name+'-previous-repo','push',name,RUN/'previous-repo','/opt/limeos-previous-repo')
                run(name+'-previous-provenance','push',name,RUN/'upgrade-inputs/schema8/previous-artifact.json','/root/qual/previous-artifact.json')
                command = 'python3 /root/qual/fixtures/tests/qualification/arm64/upgrade_guest.py --repo /opt/limeos-repo --expected /root/qual/build-result.json --previous-repo /opt/limeos-previous-repo --previous-provenance /root/qual/previous-artifact.json --output '+output
            else:
                script = 'install_guest.py' if suite=='install' else 'approved_guest.py --suite '+suite
                command = 'python3 /root/qual/fixtures/tests/qualification/arm64/'+script+' --repo /opt/limeos-repo --expected /root/qual/build-result.json --hashes /root/werkzeug-hashes.json --output '+output
            exit_code = run(name+'-suite','exec',name,command,required=False)
            if suite == 'storage' and exit_code == 0:
                paths += ['/root/qual/extra-result.json']
                run(name+'-extra','exec',name,'python3 /root/qual/fixtures/tests/qualification/arm64/extra_guest.py --expected /root/qual/build-result.json --output /root/qual/extra-result.json',required=False)
            collect(name,paths)
        finally:
            stop(name)
    status['summary']='completed; inspect each suite result and performance budgets'
except BaseException as error:
    status['summary']='failed'
    status['error']=repr(error)
    raise
finally:
    status['finished_utc']=datetime.datetime.now(datetime.timezone.utc).isoformat()
    save()
