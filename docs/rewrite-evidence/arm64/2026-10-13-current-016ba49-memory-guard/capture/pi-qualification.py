# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""Record each workstation invocation of the guarded Pi qualification runner."""
import argparse
import datetime
import json
import pathlib
import re
import shlex
import subprocess
import sys
import time

ROOT = pathlib.Path('/home/marc/Documents/github/lime-os')
RUN = ROOT / '.cache/arm64-qual/integrated-2026-10-12'
parser = argparse.ArgumentParser()
parser.add_argument('--label', required=True)
parser.add_argument('--run', type=pathlib.Path, default=RUN)
parser.add_argument('--host-script', type=pathlib.Path)
parser.add_argument('--host-command', type=pathlib.Path)
parser.add_argument('arguments', nargs=argparse.REMAINDER)
args = parser.parse_args()
RUN = args.run.resolve()
if not RUN.is_relative_to((ROOT / '.cache/arm64-qual/integrated-2026-10-12').resolve()):
    parser.error('run must stay inside the integrated qualification disk directory')
if not re.fullmatch(r'[a-z0-9-]+', args.label):
    parser.error('label must be a safe unique file name')
log = RUN / 'commands' / (args.label + '.txt')
log.parent.mkdir(exist_ok=True)
if log.exists():
    parser.error('command log already exists; choose a fresh label')
command = [sys.executable, str(ROOT / 'tests/qualification/arm64/native_vm.py'),
           '--host', 'holly@192.168.0.45', '--authorization', str(RUN / 'authorization-pi45.json')]
if args.host_script:
    command += ['host-exec', '--log', str(RUN / 'host-commands-pi45.jsonl'),
                'python3 -c ' + shlex.quote(args.host_script.read_text())]
elif args.host_command:
    command += ['host-exec', '--log', str(RUN / 'host-commands-pi45.jsonl'), args.host_command.read_text()]
else:
    command += args.arguments[1:] if args.arguments[:1] == ['--'] else args.arguments
record = {'label': args.label, 'started_utc': datetime.datetime.now(datetime.timezone.utc).isoformat(),
          'argv': command, 'log': str(log.relative_to(RUN))}
start = time.monotonic()
with log.open('x') as output:
    output.write('$ ' + shlex.join(command) + '\n')
    output.flush()
    process = subprocess.Popen(command, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, cwd=ROOT)
    try:
        for line in process.stdout:
            output.write(line)
            output.flush()
            print(line, end='', flush=True)
        code = process.wait()
    except BaseException:
        process.terminate()
        try:
            process.wait(timeout=10)
        except subprocess.TimeoutExpired:
            process.kill()
            process.wait()
        raise
record.update(exit=code, seconds=round(time.monotonic()-start, 3))
with (RUN / 'commands.jsonl').open('a') as stream:
    stream.write(json.dumps(record) + '\n')
sys.exit(code)
