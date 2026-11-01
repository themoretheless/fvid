import subprocess,os,json,hashlib
from pathlib import Path
q=Path(__file__).parent
env=os.environ.copy();env.update(CARGO_TARGET_DIR='/private/tmp/fvid-audio-eof-seek-target-20261101',CARGO_BUILD_JOBS='2',CARGO_PROFILE_DEV_DEBUG='0',CARGO_PROFILE_TEST_DEBUG='0')
old=json.loads((q/'integration-result.json').read_text())['command']
base=['cargo','test','--locked','--offline','--no-default-features','--features','player']
jobs=[('final-integration',old),('root-mp4',base+['--lib','mp4','--','--test-threads=2']),('root-webm',base+['--lib','webm','--','--test-threads=2']),('audio-worker',base+['--lib','audio_thread','--','--test-threads=2']),('media-units',['cargo','test','-p','fvid-media','--locked','--offline','--no-default-features','--features','player','--lib','--','--test-threads=2']),('core-check',['cargo','check','--locked','--offline','--no-default-features']),('player-check',['cargo','check','--locked','--offline','--no-default-features','--features','player'])]
source=Path('src/playback/audio/mp4_audio.rs'); frozen=hashlib.sha256(source.read_bytes()).hexdigest();results={}
(q/'qualification-source.json').write_text(json.dumps({'production_sha256':frozen,'test_sha256':{str(p):hashlib.sha256(p.read_bytes()).hexdigest()for p in [Path('tests/mp4_aac_subtick_seek.rs'),Path('tests/mp4_audio_eof_seek_clock.rs')] }},indent=2))
for name,cmd in jobs:
 assert hashlib.sha256(source.read_bytes()).hexdigest()==frozen
 with (q/(name+'.log')).open('w') as out:r=subprocess.run(cmd,env=env,stdout=out,stderr=subprocess.STDOUT)
 results[name]={'command':cmd,'exit_code':r.returncode}
 (q/'qualification-results.json').write_text(json.dumps(results,indent=2))
 print(name,r.returncode,flush=True)
 if r.returncode:break
