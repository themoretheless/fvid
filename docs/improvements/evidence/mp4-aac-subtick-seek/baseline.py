from pathlib import Path
import subprocess,os,json,hashlib
q=Path(__file__).resolve().parent;p=Path('src/playback/audio/mp4_audio.rs');saved=p.read_bytes();original=subprocess.check_output(['git','show','HEAD:src/playback/audio/mp4_audio.rs']);(q/'before-source.json').write_text(json.dumps({'src/playback/audio/mp4_audio.rs':hashlib.sha256(original).hexdigest(),**{n:hashlib.sha256(Path(n).read_bytes()).hexdigest()for n in ['tests/mp4_aac_subtick_seek.rs','scripts/generate_mp4_aac_subtick_seek.py']}},indent=2)+'\n');(q/'base.txt').write_text(subprocess.check_output(['git','rev-parse','HEAD'],text=True));(q/'restored-eof-clock-candidate.rs.txt').write_bytes(saved)
env=os.environ.copy();env.update(CARGO_TARGET_DIR='/private/tmp/fvid-audio-eof-seek-target-20261101',CARGO_BUILD_JOBS='2',CARGO_PROFILE_DEV_DEBUG='0',CARGO_PROFILE_TEST_DEBUG='0')
cmd=['cargo','test','--locked','--offline','--no-default-features','--features','player','--test','mp4_aac_subtick_seek']
try:
 p.write_bytes(original)
 with(q/'before.log').open('wb')as f:r=subprocess.run(cmd,stdout=f,stderr=subprocess.STDOUT,env=env)
 (q/'before-result.json').write_text(json.dumps(dict(command=cmd,exit_code=r.returncode),indent=2)+'\n')
finally:
 assert p.read_bytes()==original,'foreign source changes;inspect saved candidate'
 p.write_bytes(saved)
print('Original baseline complete;EOF-clock candidate restored',r.returncode,flush=True)
