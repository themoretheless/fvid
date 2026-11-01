from pathlib import Path
import subprocess,os,json,hashlib
q=Path(__file__).resolve().parent
paths=subprocess.check_output(['git','diff','--name-only'],text=True).splitlines();assert len(paths)==11 and all(n.endswith('.rs')for n in paths)
saved={n:Path(n).read_bytes()for n in paths};published={n:subprocess.check_output(['git','show','HEAD:'+n])for n in paths}
(q/'source-paths.json').write_text(json.dumps(paths,indent=2)+'\n')
for n,b in saved.items():
 p=q/'candidate-source'/(n+'.txt');p.parent.mkdir(parents=True,exist_ok=True);p.write_bytes(b)
(q/'final-test-source.json').write_text(json.dumps({n:hashlib.sha256(Path(n).read_bytes()).hexdigest()for n in ['tests/mp4_edit_packet_admission.rs','scripts/generate_mp4_edit_packet_admission.py']},indent=2)+'\n')
(q/'published-source.json').write_text(json.dumps({n:hashlib.sha256(b).hexdigest()for n,b in published.items()},indent=2)+'\n')
env=os.environ.copy();env.update(CARGO_TARGET_DIR='/private/tmp/fvid-edit-admission-target-20261101',CARGO_BUILD_JOBS='2',CARGO_PROFILE_TEST_DEBUG='0',CARGO_PROFILE_DEV_DEBUG='0')
cmd=['cargo','test','--locked','--offline','--no-default-features','--features','player','--test','mp4_edit_packet_admission']
try:
 for n,b in published.items():Path(n).write_bytes(b)
 with(q/'before-complete.log').open('wb')as f:r=subprocess.run(cmd,stdout=f,stderr=subprocess.STDOUT,env=env)
 (q/'before-complete-result.json').write_text(json.dumps(dict(command=cmd,exit_code=r.returncode),indent=2)+'\n')
finally:
 assert all(Path(n).read_bytes()==b for n,b in published.items()),'Foreign source changes;inspect preserved candidates before restoring'
 for n,b in saved.items():Path(n).write_bytes(b)
print('Published baseline terminal,candidate restored:',r.returncode,flush=True)
