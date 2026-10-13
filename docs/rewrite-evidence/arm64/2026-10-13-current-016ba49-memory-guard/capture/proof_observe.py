"""Read exact deadline owner/child identities and preserve an unrelated service."""
import json
import pathlib
import subprocess
import time
CONTROL = '/tmp/limeos-arm64-supervisor-PYDx8erm'
MARKER = 'limeos-arm64-integrated-proof-20261012-95495f6306f353dd'

def read_control(name):
    return subprocess.check_output(['sudo','-n','cat',CONTROL+'/'+name], text=True)

def identity(pid):
    try:
        root = pathlib.Path('/proc')/str(pid)
        stat = (root/'stat').read_text()
        fields = stat[stat.rindex(')')+2:].split()
        uid = int(next(line.split()[1] for line in (root/'status').read_text().splitlines() if line.startswith('Uid:')))
        return {'pid':pid, 'state':fields[0], 'parent_pid':int(fields[1]), 'start_ticks':int(fields[19]), 'uid':uid, 'argv':[part.decode(errors='replace') for part in (root/'cmdline').read_bytes().split(b'\0') if part]}
    except (OSError, ValueError):
        return None

owner = json.loads(read_control('owner.json'))
child = json.loads(read_control('process.json'))
assert owner['marker'] == child['marker'] == MARKER
pid = int(subprocess.check_output(['systemctl','show','pi-health','--property=MainPID','--value'],text=True))
production = identity(pid)
assert production['pid'] == 2758 and production['start_ticks'] == 3029
result = {'captured_epoch':time.time(), 'recorded_owner':owner,'recorded_child':child,'actual_owner':identity(owner['pid']),'actual_child':identity(child['pid']),'production_identity':production,'supervisor_log':[json.loads(line) for line in read_control('supervisor.log').splitlines()], 'protected_hashes':subprocess.check_output(['sudo','-n','sha256sum',CONTROL+'/guest_supervisor.py',CONTROL+'/launch.json'],text=True)}
print(json.dumps(result, indent=2))
