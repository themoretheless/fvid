import os,json,hashlib,subprocess
from pathlib import Path
q=Path(__file__).parent;src=Path('src/playback/audio/mp4_audio.rs')
backup=Path('/private/tmp/fvid-constant-edit-candidate-source-20261102.rs');assert not backup.exists()
frozen=json.loads((q/'qualification-source.json').read_text())
def verify():
 for p,h in frozen.items():assert hashlib.sha256(Path(p).read_bytes()).hexdigest()==h,p
verify();candidate=src.read_bytes();before=json.loads((q/'before-source.json').read_text());original=subprocess.check_output(['git','show',before['head']+':'+str(src)]);assert hashlib.sha256(original).hexdigest()==before['sha256'][str(src)]
stamp='tests/mp4_constant_edit_stamp_grid.rs';stamps={stamp:hashlib.sha256(Path(stamp).read_bytes()).hexdigest(),str(src):hashlib.sha256(candidate).hexdigest()};(q/'stamp-source.json').write_text(json.dumps(stamps,indent=2)+'\n')
cmd=['cargo','test','--locked','--offline','--no-default-features','--features','player','--test','mp4_constant_edit_stamp_grid']
env=os.environ.copy();env.update(CARGO_TARGET_DIR='/private/tmp/fvid-constant-edit-target-20261102',CARGO_BUILD_JOBS='2',CARGO_PROFILE_DEV_DEBUG='0',CARGO_PROFILE_TEST_DEBUG='0')
result=dict(command=cmd,baseline_head=before['head'],backup=str(backup),status='baseline-source-swap')
def save(): (q/'stamp-results.json').write_text(json.dumps(result,indent=2)+'\n')
def run(name):
 with(q/name).open('w')as log:return subprocess.run(cmd,env=env,stdout=log,stderr=subprocess.STDOUT).returncode
backup.write_bytes(candidate);save()
try:
 src.write_bytes(original);code=run('stamps-before.log');result['before_exit_code']=code;save()
finally:
 assert src.read_bytes()==original,'Unexpected source edit;candidate backup retained;not overwriting foreign work'
 src.write_bytes(candidate);verify();result['candidate_restored']=True;save()
assert code==101 and 'test result: FAILED. 0 passed; 1 failed;'in(q/'stamps-before.log').read_text()
result['after_exit_code']=run('stamps-after.log');save();assert result['after_exit_code']==0
result['repeat_exit_codes']=[]
for i in range(1,4):
 verify();assert hashlib.sha256(Path(stamp).read_bytes()).hexdigest()==stamps[stamp]
 code=run(f'stamps-repeat-{i}.log');result['repeat_exit_codes'].append(code);save();print('stamp repeat',i,code,flush=True);assert code==0
verify();assert src.read_bytes()==backup.read_bytes();backup.unlink();result.update(status='qualified',backup_removed=True);save();print('strict packet stamps qualified',flush=True)
