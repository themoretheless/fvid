from pathlib import Path
import json,hashlib,os,re,subprocess
q=Path(__file__).resolve().parent
for n,h in json.loads((q/'batch-source.json').read_text()).items():assert hashlib.sha256(Path(n).read_bytes()).hexdigest()==h
names=['mp4_mace_export','mp4_mace_composition_runs','mp4_mace_transitions','mp4_adpcm_transitions','mp4_geometry_lossless_adpcm','mp4_pcm_choice','owned_ima4','adpcm_player_owned']
cmd=['cargo','test','--locked','--offline','--no-default-features','--features','player','--no-fail-fast']+sum((['--test',n]for n in names),[])
env=os.environ.copy();env.update(CARGO_TARGET_DIR='/private/tmp/fvid-edit-admission-target-20261101',CARGO_BUILD_JOBS='2',CARGO_PROFILE_TEST_DEBUG='0',CARGO_PROFILE_DEV_DEBUG='0')
with(q/'immediate-codecs.log').open('wb')as f:r=subprocess.run(cmd,stdout=f,stderr=subprocess.STDOUT,env=env)
counts=re.findall(r'test result: (?:ok|FAILED)\. (\d+) passed; (\d+) failed; (\d+) ignored;',(q/'immediate-codecs.log').read_text());v=dict(command=cmd,exit_code=r.returncode,passed=sum(int(x[0])for x in counts),failed=sum(int(x[1])for x in counts),ignored=sum(int(x[2])for x in counts));p=q/'qualification-results.json';results=json.loads(p.read_text());assert len(results)==7 and all(v['exit_code']==0 for v in results.values());results['immediate-codecs']=v;p.write_text(json.dumps(results,indent=2)+'\n');print(v,flush=True);assert r.returncode==0
