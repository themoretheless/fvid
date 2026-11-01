from pathlib import Path
import subprocess,os,json,hashlib
q=Path(__file__).resolve().parent;source=Path('src/native_audio_mix/concat.rs');candidate=source.read_bytes();published=subprocess.check_output(['git','show','HEAD:src/native_audio_mix/concat.rs'])
(q/'candidate-concat.rs.txt').write_bytes(candidate)
(q/'final-test-source.json').write_text(json.dumps({n:hashlib.sha256(Path(n).read_bytes()).hexdigest()for n in ['tests/mp4_concat_edit_work.rs','scripts/generate_mp4_concat_edit_work.py']},indent=2)+'\n')
env=os.environ.copy();env.update(CARGO_TARGET_DIR='/private/tmp/fvid-concat-edit-work-target-20261101',CARGO_BUILD_JOBS='2',CARGO_PROFILE_TEST_DEBUG='0',CARGO_PROFILE_DEV_DEBUG='0')
cmd=['cargo','test','--locked','--offline','--no-default-features','--features','player','--test','mp4_concat_edit_work']
try:
 source.write_bytes(published)
 (q/'published-source.json').write_text(json.dumps({'src/native_audio_mix/concat.rs':hashlib.sha256(published).hexdigest()},indent=2)+'\n')
 with(q/'before-complete.log').open('wb')as f:r=subprocess.run(cmd,stdout=f,stderr=subprocess.STDOUT,env=env)
 (q/'before-complete-result.json').write_text(json.dumps(dict(command=cmd,exit_code=r.returncode),indent=2)+'\n')
finally:
 assert source.read_bytes()==published,'foreign source change;preserve and inspect'
 source.write_bytes(candidate)
print('Published baseline finished,candidate restored',r.returncode,flush=True)
