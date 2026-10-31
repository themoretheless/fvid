"""Run the frozen six acceptance cases twenty times after qualification."""
from pathlib import Path
import subprocess,json,re,hashlib
q=Path(__file__).resolve().parent
for n,h in json.loads((q/'after-source.json').read_text()).items():
    assert hashlib.sha256(Path(n).read_bytes()).hexdigest()==h,n
raw=(q/'qualification/root-mp4.log').read_text()
m=re.search(r'Running unittests src/lib.rs \(([^)]+)\)',raw)
assert m,raw[-500:]
binary=Path(m.group(1));assert binary.is_file()
results=[]
with (q/'repeat.log').open('wb') as log:
    for i in range(20):
        result=subprocess.run([str(binary),'ab_clock_regressions','--test-threads=2'],stdout=subprocess.PIPE,stderr=subprocess.STDOUT)
        log.write(('run '+str(i+1)+'\n').encode()+result.stdout);log.flush()
        assert result.returncode==0,result.stdout.decode(errors='replace')
        assert re.search(rb'test result: ok\. 6 passed; 0 failed;',result.stdout),result.stdout
        results.append({'run':i+1,'exit_code':result.returncode,'passed':6,'failed':0})
(q/'repeat.json').write_text(json.dumps({'runs':results,'total_case_executions':120,'excluded_from_distinct_checks':True,'binary_sha256':hashlib.sha256(binary.read_bytes()).hexdigest()},indent=2)+'\n')
print('20 runs;120 passing case executions',flush=True)
