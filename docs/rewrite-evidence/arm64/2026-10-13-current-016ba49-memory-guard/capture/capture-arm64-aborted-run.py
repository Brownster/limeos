# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""Record only available aborted-run evidence; exclude every private SSH key."""
import hashlib
import json
import pathlib
import shutil

ROOT = pathlib.Path('/home/marc/Documents/github/lime-os')
RUN = ROOT/'.cache/arm64-qual/integrated-2026-10-12'
OUT = ROOT/'docs/rewrite-evidence/arm64/2026-10-13-current-016ba49-memory-guard'
assert not OUT.exists()
OUT.mkdir()
def copy(source,destination):
    target=OUT/destination
    target.parent.mkdir(parents=True,exist_ok=True)
    if source.is_dir():
        shutil.copytree(source,target)
    else:
        shutil.copy2(source,target)

for name in ['authorization.json','authorization-pi45-original.json','authorization-pi45.json','host-package-decision.json','host-preparation-plan.json','host-preflight.json','host-baseline.json','host-commands.jsonl','host-commands-pi45.jsonl','host-transfers.jsonl','preparation-before.json','preparation-after.json','preparation-install.txt','host-packages-proposed.txt','host-packages-before.txt','host-packages-after-install.txt','ci-main-status.json','source-manifest.json','deadline-proof-result.json','debian-SHA512SUMS']:
    copy(RUN/name,'inputs/'+name)
for name in ['commands','sequence-logs','sequence-status.json','commands.jsonl','host-samples','attempt-1-controller','attempt-1-status-note.json']:
    copy(RUN/name,'attempt-1/'+name)
low=RUN/'low-memory-1'
for name in ['commands','sequence-logs','sequence-status.json','commands.jsonl','host-samples','host-commands-pi45.jsonl','retry-decision.json','host_sample.py']:
    copy(low/name,'attempt-2/'+name)
allowed=['guest.json','launch.json','host-before.json','host-after.json','console.log','supervisor.log']
for guest,label in [('integrated-proof-20261012','deadline-proof'),('integrated-build-20261012','attempt-1/guest'),('integrated-lowmem-build-20261012','attempt-2/guest')]:
    directory=ROOT/'.cache/arm64-qual/stopped'/guest
    for name in allowed:
        copy(directory/name,label+'/'+name)
for name in ['run.json','artifact.json','job.json','verified-inputs.json']:
    copy(RUN/'native-ci-inputs'/name,'next-inputs/native-ci/'+name)
for name in ['candidate-provenance.json','standard-package.txt','shadow-package.txt']:
    copy(RUN/'ci-candidate-0.4.8-arm64.3'/name,'next-inputs/candidate/'+name)
for name in ['pi-qualification.py','pi-qualification-low-memory.py','pi-host-transfer.py','prepare-arm64-ci-inputs.py','package-arm64-ci-candidate.py','verify-pi-deadline-proof.py','capture-arm64-aborted-run.py']:
    copy(ROOT/'.cache'/name,'capture/'+name)
for name in ['prepare_host.py','prepare-image-command.txt','verify_host_image.py','proof_observe.py','host_baseline.py','host_sample.py','host_memory_diagnosis.py']:
    copy(RUN/name,'capture/'+name)
copy(RUN/'upgrade-inputs/schema8/previous-artifact.json','inputs/previous-artifact.json')

baseline=json.loads((RUN/'host-baseline.json').read_text())
final=json.loads((low/'host-samples/014.json').read_text())
def containers(data):
    return {parts[0]:{'name':parts[1],'state':parts[2]} for line in data['containers']['stdout'].splitlines() if (parts:=line.split('\t'))}
assert containers(baseline)==containers(final)
assert baseline['boot_id']==final['boot_id']
def services(text):
    return {record['Id']:record for group in text.split('\n\n') if group.strip() and (record:=dict(line.split('=',1) for line in group.strip().splitlines()))}
before=services(baseline['services']['stdout'])
after=services(final['services']['stdout'])
for name,record in after.items():
    for field in ['MainPID','NRestarts','ActiveState','SubState']:
        assert record[field]==before[name][field], (name,field)
assert not final['test_processes'] and final['vmstat']['oom_kill']==0
assert all(value['status']==200 for value in final['http'].values())
snapshot=RUN/'ci-candidate-0.4.8-arm64.3/source'
manifest=json.loads((ROOT/'docs/rewrite-evidence/p04/2026-10-12-completed-slices/source-manifest.json').read_text())
for name,digest in manifest['files'].items():
    assert hashlib.sha256((snapshot/name).read_bytes()).hexdigest()==digest, name
comparison={'integration_source':manifest['qualified_source'],'ci_source':'af94f19ac21f9da14b207439808ffb33c87e8e10','matching_source_fixture_contract_script_inputs':len(manifest['files']),'files':manifest['files']}
(OUT/'next-inputs/compiled-input-comparison.json').write_text(json.dumps(comparison,indent=2)+'\n')
identity=json.loads((RUN/'source-manifest.json').read_text())['identity']
summary={'identity':identity,'status':'incomplete: host swap guard stopped both native build attempts','deadline_proof':'pass','native_build_gates_completed_in_retry':['apt','rustup','fetch','fmt','clippy'],'native_workspace_tests_completed':False,'native_packages_created':False,'untested_gates':['current-candidate installed ARM64 behavior','genuine schema-8 to schema-9 ARM64 upgrade','current-candidate storage/container interruption recovery','current-candidate footprint and latency','bare-metal 16 KiB kernel qualification','hardware power interruption and soak'],'last_post_teardown_health':{'captured_utc':final['captured_utc'],'boot_id_unchanged':True,'running_containers_preserved':14,'preexisting_stopped_containers_preserved':4,'service_pids_and_restart_counts_unchanged':True,'http':final['http'],'test_processes':[],'new_oom_kills':0,'available_ram_mib':6005,'swap_mib':449,'temperature_c':55.1},'baseline_conditions':{'swap_mib':round((524272-217440)/1024,2),'failed_unit':'docker-compose-start.service','stopped_containers':['airsonic-advanced','limeos-mattermost-db','limeos-alertd','limeos-mattermost']},'retained_host_setup':'Only the approved QEMU/firmware/dependency set and verified base image/preparation records; no guest or supervisor remains in the last post-test sample','missing_evidence':'Safety teardown deleted both overlays before full guest build/log collection. The delayed progress snapshot was refused after expiry before SSH. Complete available coordinator, console, supervisor and host-sample evidence is retained; detailed guest test logs and partial build JSON are unavailable.','no_successful_native_guest_record':'record_run.py was not invoked: it requires a successful native build, which this run lacks','next_input_preparation':'Original native CI artifacts verified and fresh 0.4.8+arm64.3 packages prepared locally from unchanged native CI ELFs; explicit CI-provenance fixture support, fresh signing, and a new test window remain required'}
(OUT/'qualification-status.json').write_text(json.dumps(summary,indent=2)+'\n')
print(json.dumps(summary,indent=2))
