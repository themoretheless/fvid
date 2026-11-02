import json,os,subprocess,hashlib
from pathlib import Path
q=Path(__file__).parent
cmd=json.loads(Path('docs/improvements/evidence/mp4-mace-edit-subtick-seek/repeat-results.json').read_text())['command']+['--test', 'mp4_constant_edit_subtick_seek', '--test', 'mp4_adpcm_transitions', '--test', 'mp4_geometry_lossless_adpcm', '--test', 'mp4_ac3_sample_clock', '--test', 'ac3_mp4_channels', '--test', 'mp4_eac3_programme_clock', '--test', 'eac3_mp4_programmes']
env=os.environ.copy();env.update(CARGO_TARGET_DIR='/private/tmp/fvid-constant-edit-target-20261102',CARGO_BUILD_JOBS='2',CARGO_PROFILE_DEV_DEBUG='0',CARGO_PROFILE_TEST_DEBUG='0')
frozen=json.loads((q/'qualification-source.json').read_text());codes=[]
for i in range(1,4):
 assert all(hashlib.sha256(Path(p).read_bytes()).hexdigest()==h for p,h in frozen.items())
 with(q/f'repeat-{i}.log').open('w')as out:r=subprocess.run(cmd,env=env,stdout=out,stderr=subprocess.STDOUT)
 codes.append(r.returncode);(q/'repeat-results.json').write_text(json.dumps(dict(command=cmd,exit_codes=codes),indent=2)+'\n');print(i,r.returncode,flush=True)
 if r.returncode:break
