"""Replay the recorded offline qualification plan from the repository root."""
import hashlib
import json
import os
import re
import subprocess
from pathlib import Path

q = Path(__file__).resolve().parent
plan = json.loads((q / 'qualification-plan.json').read_text())
sources = json.loads((q / 'after-source.json').read_text())

def verify_sources():
    for name, expected in sources.items():
        assert hashlib.sha256(Path(name).read_bytes()).hexdigest() == expected, name

verify_sources()
env = os.environ.copy()
env.update(plan['env'])
results = {}
for kind, command in plan['commands'].items():
    verify_sources()
    log = Path(plan['logs'].format(kind=kind))
    with log.open('wb') as output:
        result = subprocess.run(command, stdout=output, stderr=subprocess.STDOUT, env=env)
    raw = log.read_text(errors='replace')
    matches = re.findall(r'test result: (?:ok|FAILED)\. (\d+) passed; (\d+) failed; (\d+) ignored;', raw)
    results[kind] = {'command': command, 'exit_code': result.returncode, 'log': str(log),
                     'suites': len(matches), 'passed': sum(int(m[0]) for m in matches),
                     'failed': sum(int(m[1]) for m in matches), 'ignored': sum(int(m[2]) for m in matches)}
    (q / 'qualification-results.json').write_text(json.dumps(results, indent=2) + '\n')
    print(kind, result.returncode, results[kind]['passed'], flush=True)
verify_sources()
assert len(results) == 6 and all(v['exit_code'] == 0 for v in results.values()), results
