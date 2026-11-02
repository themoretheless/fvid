import json,hashlib,os,subprocess
from pathlib import Path
q=Path(__file__).parent
paths=['tests/mp4_mace_transitions.rs','tests/mp4_mace_composition_runs.rs','tests/mp4_mace_export.rs','scripts/generate_mp4_mace_transitions.py','tests/support/audio_transitions.rs','src/playback/audio/mp4_audio.rs']
frozen={p:hashlib.sha256(Path(p).read_bytes()).hexdigest()for p in paths};(q/'related-source.json').write_text(json.dumps(frozen,indent=2)+'\n')
for p,h in json.loads((q/'qualification-source.json').read_text()).items():assert hashlib.sha256(Path(p).read_bytes()).hexdigest()==h
cmd=['cargo','test','--locked','--offline','--no-default-features','--features','player','--no-fail-fast','--test','mp4_mace_transitions','--test','mp4_mace_composition_runs','--test','mp4_mace_export']
env=os.environ.copy();env.update(CARGO_TARGET_DIR='/private/tmp/fvid-mace-subtick-target-20261102',CARGO_BUILD_JOBS='2',CARGO_PROFILE_DEV_DEBUG='0',CARGO_PROFILE_TEST_DEBUG='0')
with(q/'related.log').open('w')as out:r=subprocess.run(cmd,env=env,stdout=out,stderr=subprocess.STDOUT)
assert all(hashlib.sha256(Path(p).read_bytes()).hexdigest()==h for p,h in frozen.items())
(q/'related-results.json').write_text(json.dumps(dict(command=cmd,exit_code=r.returncode),indent=2)+'\n');print('related',r.returncode,flush=True)
