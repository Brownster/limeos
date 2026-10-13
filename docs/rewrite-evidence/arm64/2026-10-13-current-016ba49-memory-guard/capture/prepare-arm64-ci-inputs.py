# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""Collect and verify existing native ARM64 CI packages without contacting the Pi."""
import hashlib
import json
import pathlib
import subprocess
import tarfile
import zipfile

ROOT = pathlib.Path('/home/marc/Documents/github/lime-os')
RUN = ROOT / '.cache/arm64-qual/integrated-2026-10-12/native-ci-inputs'
RUN.mkdir(exist_ok=False)
REPO = 'Brownster/limeos'
RUN_ID = 38302065106
ARTIFACT_ID = 11770788524
SOURCE = 'af94f19ac21f9da14b207439808ffb33c87e8e10'
def api(path):
    return json.loads(subprocess.check_output(['gh','api',path],text=True))
run = api(f'repos/{REPO}/actions/runs/{RUN_ID}')
artifact = api(f'repos/{REPO}/actions/artifacts/{ARTIFACT_ID}')
jobs = api(f'repos/{REPO}/actions/runs/{RUN_ID}/jobs?filter=latest&per_page=100')
assert run['head_sha']==SOURCE and run['conclusion']=='success' and run['status']=='completed'
assert artifact['name']=='debian-ubuntu-24.04-arm' and not artifact['expired']
assert artifact['workflow_run']['head_sha']==SOURCE
job = next(job for job in jobs['jobs'] if job['name']=='check (ubuntu-24.04-arm)')
assert job['conclusion']=='success'
for data,name in [(run,'run.json'),(artifact,'artifact.json'),(job,'job.json')]:
    (RUN/name).write_text(json.dumps(data,indent=2)+'\n')
archive = RUN/'artifact.zip'
with archive.open('wb') as stream:
    subprocess.run(['gh','api',f'repos/{REPO}/actions/artifacts/{ARTIFACT_ID}/zip'],stdout=stream,check=True)
digest = hashlib.file_digest(archive.open('rb'),'sha256').hexdigest()
assert artifact['digest']=='sha256:'+digest, (digest,artifact['digest'])
expected = {'limeos_0.4.4_arm64.deb','limeos_0.4.4+ci.1_arm64.deb','limeos-shadow_0.4.4_arm64.deb'}
packages = RUN/'packages'
packages.mkdir()
with zipfile.ZipFile(archive) as source:
    assert set(source.namelist())==expected, source.namelist()
    for name in sorted(expected):
        data=source.read(name)
        assert len(data)<32*1024*1024
        (packages/name).write_bytes(data)
names = ['limeos-core','limeos-password-worker','limeos-executor','limeosctl']
records = {}
for path in sorted(packages.glob('*.deb')):
    control=subprocess.check_output(['dpkg-deb','-f',str(path),'Package','Version','Architecture'],text=True)
    profile='limeos-shadow' if path.name.startswith('limeos-shadow_') else 'limeos'
    version='0.4.4+ci.1' if '+ci.1' in path.name else '0.4.4'
    assert control==f'Package: {profile}\nVersion: {version}\nArchitecture: arm64\n'
    process=subprocess.Popen(['dpkg-deb','--fsys-tarfile',str(path)],stdout=subprocess.PIPE)
    binaries={}
    with tarfile.open(fileobj=process.stdout,mode='r|') as payload:
        for member in payload:
            if member.name not in [f'./usr/lib/{profile}/{name}' for name in names]:
                continue
            assert member.isfile() and member.size<16*1024*1024
            name=pathlib.PurePosixPath(member.name).name
            data=payload.extractfile(member).read()
            assert data[:7]==b'\x7fELF\x02\x01\x01' and int.from_bytes(data[18:20],'little')==183
            binaries[name]={'bytes':len(data),'sha256':hashlib.sha256(data).hexdigest(),'elf_machine':183}
    assert process.wait()==0 and set(binaries)==set(names)
    records[path.name]={'bytes':path.stat().st_size,'sha256':hashlib.file_digest(path.open('rb'),'sha256').hexdigest(),'control':control,'binaries':binaries}
reference = next(iter(records.values()))['binaries']
assert all(record['binaries']==reference for record in records.values())
summary={'source_commit':SOURCE,'workflow_run_id':RUN_ID,'workflow_url':run['html_url'],'native_arm64_job_id':job['id'],'native_arm64_job_url':job['html_url'],'artifact_id':ARTIFACT_ID,'artifact_zip_sha256':digest,'packages':records,'pi_accessed':False,'pi_install_upgrade_crash_or_footprint_qualified':False,'limitations':['These original current-source CI packages are version 0.4.4, not tonight\'s 0.4.8+arm64.2 candidate.','Version 0.4.4 is older than the genuine schema-8 version 0.4.7+arm64.1; do not count direct installation as an upgrade.','A separately recorded fresh candidate must bind the original native CI ELF hashes, actual source/build/package inputs, fixture identity and new test signing repository.','The current guest recorder requires a successful native guest build; CI evidence needs an explicit supported provenance path, never fabricated guest build steps.']}
(RUN/'verified-inputs.json').write_text(json.dumps(summary,indent=2)+'\n')
print(json.dumps({key:value for key,value in summary.items() if key!='packages'},indent=2))
