# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""Package unchanged native CI ELFs with frozen source; never modify prior artifacts."""
import hashlib
import json
import os
import pathlib
import re
import subprocess
import sys
import tarfile

ROOT = pathlib.Path('/home/marc/Documents/github/lime-os')
BASE = ROOT / '.cache/arm64-qual/integrated-2026-10-12'
INPUT = BASE / 'native-ci-inputs'
RUN = BASE / 'ci-candidate-0.4.8-arm64.3'
RUN.mkdir(exist_ok=False)
proof = json.loads((INPUT/'verified-inputs.json').read_text())
SOURCE = proof['source_commit']
assert SOURCE == 'af94f19ac21f9da14b207439808ffb33c87e8e10'
with (RUN/'source.tar').open('wb') as stream:
    subprocess.run(['git','archive','--format=tar',SOURCE],cwd=ROOT,stdout=stream,check=True)
source = RUN/'source'
source.mkdir()
with tarfile.open(RUN/'source.tar') as archive:
    archive.extractall(source,filter='data')
schema = int(re.search(r'pub const SCHEMA_VERSION: u32 = (\d+);',(source/'crates/persistence/src/lib.rs').read_text())[1])
assert schema == 9
binary_dir = RUN/'binaries'
binary_dir.mkdir()
package = INPUT/'packages/limeos_0.4.4_arm64.deb'
package_proof = proof['packages'][package.name]
with package.open('rb') as stream:
    assert hashlib.file_digest(stream,'sha256').hexdigest() == package_proof['sha256']
process = subprocess.Popen(['dpkg-deb','--fsys-tarfile',str(package)],stdout=subprocess.PIPE)
frontend = {}
contracts = {}
names = set(package_proof['binaries'])
selected = set()
with tarfile.open(fileobj=process.stdout,mode='r|') as payload:
    for member in payload:
        prefix = './usr/lib/limeos/'
        if not member.name.startswith(prefix):
            continue
        name = member.name[len(prefix):]
        if member.isdir():
            continue
        if name not in names and not name.startswith(('ui/','contracts/')):
            continue
        assert member.isfile() and member.size < 16*1024*1024
        relative = pathlib.PurePosixPath(name)
        assert '..' not in relative.parts and not relative.is_absolute()
        data = payload.extractfile(member).read()
        digest = hashlib.sha256(data).hexdigest()
        if name in names:
            assert digest == package_proof['binaries'][name]['sha256']
            target = binary_dir/name
            target.write_bytes(data)
            target.chmod(0o755)
            selected.add(name)
        elif name.startswith('ui/'):
            relative = pathlib.PurePosixPath(name[3:])
            target = source/'frontend/dist'/relative
            target.parent.mkdir(parents=True,exist_ok=True)
            target.write_bytes(data)
            frontend[str(relative)] = digest
        else:
            relative = pathlib.PurePosixPath(name[len('contracts/'):])
            target = source/'contracts/generated'/relative
            assert hashlib.sha256(target.read_bytes()).hexdigest() == digest, str(relative)
            contracts[str(relative)] = digest
assert process.wait()==0 and selected==names
assert 'index.html' in frontend and contracts
VERSION = '0.4.8+arm64.3'
packages = RUN/'packages'
steps=[]
for profile in ['standard','shadow']:
    command=[sys.executable,str(source/'packaging/build.py'),'--arch','arm64','--binaries',str(binary_dir),'--version',VERSION,'--profile',profile,'--output',str(packages)]
    with (RUN/(profile+'-package.txt')).open('w') as log:
        result = subprocess.run(command,stdout=log,stderr=subprocess.STDOUT,env=os.environ)
    steps.append({'profile':profile,'argv':command,'exit':result.returncode})
    assert result.returncode==0, profile
records={}
for path in sorted(packages.glob('*.deb')):
    profile='limeos-shadow' if path.name.startswith('limeos-shadow_') else 'limeos'
    control=subprocess.check_output(['dpkg-deb','-f',str(path),'Package','Version','Architecture'],text=True)
    assert control==f'Package: {profile}\nVersion: {VERSION}\nArchitecture: arm64\n'
    process=subprocess.Popen(['dpkg-deb','--fsys-tarfile',str(path)],stdout=subprocess.PIPE)
    matched={}
    with tarfile.open(fileobj=process.stdout,mode='r|') as payload:
        for member in payload:
            prefix=f'./usr/lib/{profile}/'
            name=member.name[len(prefix):] if member.name.startswith(prefix) else ''
            if name in names:
                assert member.isfile()
                data=payload.extractfile(member).read()
                matched[name]=hashlib.sha256(data).hexdigest()
                assert matched[name]==package_proof['binaries'][name]['sha256']
    assert process.wait()==0 and set(matched)==names
    records[path.name]={'bytes':path.stat().st_size,'sha256':hashlib.file_digest(path.open('rb'),'sha256').hexdigest(),'control':control,'binary_sha256':matched}
assert len(records)==2
previous=BASE/'upgrade-inputs/schema8/limeos_0.4.7+arm64.1_arm64.deb'
assert hashlib.file_digest(previous.open('rb'),'sha256').hexdigest()=='6c86199b253e155c94d96e1395c069c4188e11a2379f14eff97eacab0203b898'
assert subprocess.run(['dpkg','--compare-versions',VERSION,'gt','0.4.7+arm64.1']).returncode==0
record={'format_version':1,'build_kind':'native-ci-elfs-local-packaging','source_commit':SOURCE,'package_version':VERSION,'architecture':'arm64','expected_authority_schema':schema,'original_ci_evidence':proof,'package_builder_sha256':hashlib.sha256((source/'packaging/build.py').read_bytes()).hexdigest(),'frontend_dist_sha256':frontend,'contracts_sha256':contracts,'binaries':package_proof['binaries'],'packaging_steps':steps,'packages':records,'pi_accessed':False,'native_guest_build_qualified':False,'installed_behavior_qualified':False,'signed_test_repository_prepared':False,'qualification_fixture_provenance':'Pending explicit CI provenance support; do not feed this record to the native-guest build recorder','previous_artifact_unchanged':True}
(RUN/'candidate-provenance.json').write_text(json.dumps(record,indent=2)+'\n')
print(json.dumps({key:value for key,value in record.items() if key not in ('original_ci_evidence','contracts_sha256','packaging_steps')},indent=2))
