"""Compare unchanged source/freeze tests with pre0463 code;restore current bytes."""
from pathlib import Path
import subprocess,json,hashlib,difflib,os,re
q=Path(__file__).resolve().parent
current=Path('src/player.rs').read_bytes();expected=json.loads((q/'before-source.json').read_text())
for n,h in expected.items():assert hashlib.sha256(Path(n).read_bytes()).hexdigest()==h,n
baseline=(q/'before.log').read_text()
assert re.search(r'test result: FAILED\. 2 passed; 4 failed;',baseline),'confirm the actual current failure before this comparison'
assert 'read did not actually block' not in baseline,'fixture admission is not a clock failure'
assert 'silent media clock advances while actual input is buffered' in baseline
assert 'stop450ms consumed while' in baseline
old_commit='9ca4c32089c216f82574c0895da3a0bdbbd76c57'
old=subprocess.check_output(['git','show',old_commit+':src/player.rs'])
hook=b'\n#[cfg(test)]\n#[path = "player_io_clock_tests.rs"]\nmod io_clock_regressions;\n';temporary=old+hook
backup=q/'current-source-backup.rs.txt';backup.write_bytes(current)
(q/'pre463.patch').write_text(''.join(difflib.unified_diff(old.decode().splitlines(True),temporary.decode().splitlines(True),fromfile='a/src/player.rs',tofile='b/src/player.rs',n=0)))
(q/'pre463-source.json').write_text(json.dumps({**expected,'src/player.rs':hashlib.sha256(temporary).hexdigest()},indent=2)+'\n')
env=os.environ.copy();env.update(CARGO_TARGET_DIR='/private/tmp/fvid-player-io-clock-target-20261101',CARGO_BUILD_JOBS='2',CARGO_PROFILE_TEST_DEBUG='0',CARGO_PROFILE_DEV_DEBUG='0')
results={'old_commit':old_commit,'unchanged_test_sha256':expected['src/player_io_clock_tests.rs'],'case_scope':'Full identical six-case diagnostic on pre0463 source. Existing terminal-stop timing failures and recovery failures must remain visible. This is not an acceptance gate for old code.','cases':[]}

try:
 Path('src/player.rs').write_bytes(temporary)
 command=['cargo','test','--locked','--offline','--no-default-features','--features','player','--lib','io_clock_regressions','--','--test-threads=2']
 with(q/'pre463-full.log').open('wb')as log:r=subprocess.run(command,env=env,stdout=log,stderr=subprocess.STDOUT)
 text=(q/'pre463-full.log').read_text(errors='replace');outcomes={name:status for name,status in re.findall(r'^test player::io_clock_regressions::(\S+) \.\.\. (ok|FAILED)$',text,re.M)}
 results.update(command=command,exit_code=r.returncode,outcomes=outcomes)
 print('pre0463 full diagnostic',r.returncode,flush=True);print('\n'.join(text.splitlines()[-45:]),flush=True)

finally:
 assert Path('src/player.rs').read_bytes()==temporary,'Concurrent source change;preserve backup and do not overwrite'
 Path('src/player.rs').write_bytes(current);results['current_source_restored']=hashlib.sha256(Path('src/player.rs').read_bytes()).hexdigest()==expected['src/player.rs']
 (q/'pre463-comparison.json').write_text(json.dumps(results,indent=2)+'\n');assert results['current_source_restored'];backup.unlink()
assert len(results['outcomes'])==6,results
assert results['outcomes']['exact_mp4_io_stall_source_pixels_pts_seek_and_eof_control']=='ok',results
assert results['outcomes']['real_read_stall_freezes_media_time_then_resumes_without_rewinding']=='ok',results
assert results['outcomes']['input_recovery_clock_never_rewinds_before_the_next_picture_is_shown']=='FAILED',results
print('old freeze passes but recovery rewinds;current source restored',flush=True)
