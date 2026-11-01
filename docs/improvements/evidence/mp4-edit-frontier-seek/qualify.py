import subprocess,os,json,hashlib
from pathlib import Path
q=Path(__file__).parent
env=os.environ.copy();env.update(CARGO_TARGET_DIR='/private/tmp/fvid-edit-frontier-merge-target-20261102',CARGO_BUILD_JOBS='2',CARGO_PROFILE_DEV_DEBUG='0',CARGO_PROFILE_TEST_DEBUG='0')
old=json.loads(Path('docs/improvements/evidence/mp4-aac-native-grid-seek/qualification-results.json').read_text())['integration']['command']+['--test','mp4_edit_frontier_seek','--test','mp4_audio_metadata_admission','--test','mp4_metadata_admission']
base=['cargo','test','--locked','--offline','--no-default-features','--features','player']
jobs=[('integration',old),('root-mp4',base+['--lib','mp4','--','--test-threads=2']),('root-webm',base+['--lib','webm','--','--test-threads=2']),('audio-worker',base+['--lib','audio_thread','--','--test-threads=2']),('media-units',['cargo','test','-p','fvid-media','--locked','--offline','--no-default-features','--features','player','--lib','--','--test-threads=2']),('core-check',['cargo','check','--locked','--offline','--no-default-features']),('player-check',['cargo','check','--locked','--offline','--no-default-features','--features','player'])]
paths=[Path(p)for p in ['src/playback/audio/mp4_audio.rs','src/playback/audio/mp4_clock_recovery.rs','tests/mp4_edit_frontier_seek.rs','scripts/generate_mp4_edit_frontier_seek.py']];paths += [Path(p)for p in subprocess.check_output(['git','diff','--name-only','aef465fb75db78501c794a0eb97b18112732299f','5e8814e76cc0760a90e8f2874cc8e3cccbc7554f'],text=True).splitlines()if p.endswith('.rs')and not p.startswith('docs/')];frozen={str(p):hashlib.sha256(p.read_bytes()).hexdigest()for p in paths};(q/'qualification-source.json').write_text(json.dumps(frozen,indent=2)+'\n');results={}
for name,cmd in jobs:
 assert all(hashlib.sha256(Path(p).read_bytes()).hexdigest()==h for p,h in frozen.items())
 with(q/(name+'.log')).open('w')as out:r=subprocess.run(cmd,env=env,stdout=out,stderr=subprocess.STDOUT)
 results[name]={'command':cmd,'exit_code':r.returncode};(q/'qualification-results.json').write_text(json.dumps(results,indent=2)+'\n');print(name,r.returncode,flush=True)
 if r.returncode:break
