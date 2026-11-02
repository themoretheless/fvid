import subprocess,os,json,hashlib
from pathlib import Path
q=Path(__file__).parent
env=os.environ.copy();env.update(CARGO_TARGET_DIR='/private/tmp/fvid-unedited-alac-target-20261102',CARGO_BUILD_JOBS='2',CARGO_PROFILE_DEV_DEBUG='0',CARGO_PROFILE_TEST_DEBUG='0')
old=json.loads(Path('docs/improvements/evidence/mp4-unedited-pcm-subtick-seek/qualification-results.json').read_text())['integration']['command']+['--test','mp4_unedited_alac_subtick_seek','--test','mp4_unedited_alac_truncated_seek']
base=['cargo','test','--locked','--offline','--no-default-features','--features','player']
jobs=[('integration',old),('root-mp4',base+['--lib','mp4','--','--test-threads=2']),('root-webm',base+['--lib','webm','--','--test-threads=2']),('audio-worker',base+['--lib','audio_thread','--','--test-threads=2']),('media-units',['cargo','test','-p','fvid-media','--locked','--offline','--no-default-features','--features','player','--lib','--','--test-threads=2']),('core-check',['cargo','check','--locked','--offline','--no-default-features']),('player-check',['cargo','check','--locked','--offline','--no-default-features','--features','player'])]
paths=[Path(p)for p in json.loads(Path('docs/improvements/evidence/mp4-unedited-pcm-subtick-seek/qualification-source.json').read_text())]+[Path('tests/mp4_unedited_alac_subtick_seek.rs'),Path('tests/mp4_unedited_alac_truncated_seek.rs'),Path('scripts/generate_mp4_unedited_alac_subtick_seek.py')];frozen={str(p):hashlib.sha256(p.read_bytes()).hexdigest()for p in paths};(q/'qualification-source.json').write_text(json.dumps(frozen,indent=2)+'\n');results={}
for name,cmd in jobs:
 assert all(hashlib.sha256(Path(p).read_bytes()).hexdigest()==h for p,h in frozen.items())
 with(q/(name+'.log')).open('w')as out:r=subprocess.run(cmd,env=env,stdout=out,stderr=subprocess.STDOUT)
 results[name]={'command':cmd,'exit_code':r.returncode};(q/'qualification-results.json').write_text(json.dumps(results,indent=2)+'\n');print(name,r.returncode,flush=True)
 if r.returncode:break
