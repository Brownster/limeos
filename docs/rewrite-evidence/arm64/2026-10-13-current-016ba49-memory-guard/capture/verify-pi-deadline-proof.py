# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
import hashlib
import json
import pathlib
import shlex
ROOT = pathlib.Path('/home/marc/Documents/github/lime-os')
RUN = ROOT / '.cache/arm64-qual/integrated-2026-10-12'
command = 'python3 -c '+shlex.quote((RUN/'proof_observe.py').read_text())
records = [json.loads(line) for line in (RUN/'host-commands-pi45.jsonl').read_text().splitlines()]
records = [record for record in records if record['command']==command]
assert len(records)==2 and all(record['exit']==0 for record in records)
before, after = [json.loads(record['stdout']) for record in records]
assert before['actual_owner']['parent_pid'] == 1
assert before['actual_child']['parent_pid'] == before['actual_owner']['pid']
assert before['recorded_owner'] == after['recorded_owner']
assert before['recorded_child'] == after['recorded_child']
assert after['actual_owner'] is None and after['actual_child'] is None
assert before['production_identity'] == after['production_identity']
events = after['supervisor_log']
bound = next(event for event in events if event['event']=='bound')
term = next(event for event in events if event['event']=='sigterm')
gone = next(event for event in events if event['event']=='guest_gone')
assert term['epoch'] >= bound['terminate_at']
assert gone['epoch'] >= term['epoch'] and gone['signalled']
assert gone['epoch'] < bound['kill_at']
assert events[-1]['event'] == 'guest_gone'
supervisor = hashlib.sha256((ROOT/'tests/qualification/arm64/guest_supervisor.py').read_bytes()).hexdigest()
assert after['protected_hashes'].splitlines()[0].split()[0] == supervisor
result = {'summary':'pass','orchestrator_commit':'af94f19ac21f9da14b207439808ffb33c87e8e10','supervisor_sha256':supervisor,'proof_deadline_utc':'2026-10-12T22:55:00Z','sigterm_epoch':term['epoch'],'guest_gone_epoch':gone['epoch'],'sigkill_needed':False,'detached_owner_parent_pid':1,'unrelated_pi_health_identity_preserved':after['production_identity'],'before':before,'after':after}
(RUN/'deadline-proof-result.json').write_text(json.dumps(result,indent=2)+'\n')
print('PASS: detached host owner stopped and reaped its exact guest at the deadline; production identity unchanged')
