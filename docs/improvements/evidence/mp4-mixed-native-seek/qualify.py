import subprocess,os,json,hashlib
from pathlib import Path
q=Path(__file__).parent
env=os.environ.copy();env.update(CARGO_TARGET_DIR='/private/tmp/fvid-mixed-native-target-20261102',CARGO_BUILD_JOBS='2',CARGO_PROFILE_DEV_DEBUG='0',CARGO_PROFILE_TEST_DEBUG='0')
old=json.loads(Path('docs/improvements/evidence/mp4-constant-edit-subtick-seek/qualification-results.json').read_text())['integration']['command']+['--test', 'mp4_mixed_native_seek', '--test', 'mp4_constant_edit_stamp_grid', '--test', 'mp4_geometry_normalization']
base=['cargo','test','--locked','--offline','--no-default-features','--features','player']
jobs=[('integration',old),('root-mp4',base+['--lib','mp4','--','--test-threads=2']),('root-webm',base+['--lib','webm','--','--test-threads=2']),('audio-worker',base+['--lib','audio_thread','--','--test-threads=2']),('media-units',['cargo','test','-p','fvid-media','--locked','--offline','--no-default-features','--features','player','--lib','--','--test-threads=2']),('core-check',['cargo','check','--locked','--offline','--no-default-features']),('player-check',['cargo','check','--locked','--offline','--no-default-features','--features','player'])]
paths=[Path(p)for p in json.loads(Path('docs/improvements/batch-519-validation.json').read_text())['source_frozen']]+[Path(p)for p in ['tests/mp4_mixed_native_seek.rs', 'tests/mp4_constant_edit_stamp_grid.rs', 'tests/mp4_geometry_normalization.rs', 'scripts/generate_mp4_mixed_native_seek.py', 'scripts/generate_mp4_mace_transitions.py', 'scripts/generate_mp4_adpcm_transitions.py', 'scripts/generate_mp4_variable_rate_edits.py']];frozen={str(p):hashlib.sha256(p.read_bytes()).hexdigest()for p in paths};(q/'qualification-source.json').write_text(json.dumps(frozen,indent=2)+'\n');results={}
for name,cmd in jobs:
 assert all(hashlib.sha256(Path(p).read_bytes()).hexdigest()==h for p,h in frozen.items())
 with(q/(name+'.log')).open('w')as out:r=subprocess.run(cmd,env=env,stdout=out,stderr=subprocess.STDOUT)
 results[name]={'command':cmd,'exit_code':r.returncode};(q/'qualification-results.json').write_text(json.dumps(results,indent=2)+'\n');print(name,r.returncode,flush=True)
 if r.returncode:break
