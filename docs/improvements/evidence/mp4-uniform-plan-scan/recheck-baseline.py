from pathlib import Path
import json,hashlib,subprocess,os,shutil
q=Path(__file__).resolve().parent;names=['crates/fvid-media/src/owned_mp4_impl.rs','crates/fvid-media/src/owned_mp4_audio.rs','crates/fvid-media/src/owned_mp4_audio_export.rs'];candidate={n:Path(n).read_bytes()for n in names};published={n:subprocess.check_output(['git','show',(q/'base.txt').read_text().strip()+':'+n])for n in names};env=os.environ.copy();env.update(CARGO_TARGET_DIR='/private/tmp/fvid-uniform-plan-target-20261101',CARGO_BUILD_JOBS='2',CARGO_PROFILE_RELEASE_DEBUG='0',CARGO_PROFILE_DEV_DEBUG='0',CARGO_PROFILE_TEST_DEBUG='0')
(q/'baseline-test-source.json').write_text(json.dumps({n:hashlib.sha256(Path(n).read_bytes()).hexdigest()for n in ['tests/mp4_uniform_plan_scan.rs','scripts/generate_mp4_uniform_plan_scan.py','crates/fvid-media/examples/bench_mp4_uniform_plan_scan.rs']},indent=2)+'\n')
for n,b in candidate.items():p=q/'candidate-source'/(n+'.txt');p.parent.mkdir(parents=True,exist_ok=True);p.write_bytes(b)
try:
 for n,b in published.items():Path(n).write_bytes(b)
 cmd=['cargo','build','--release','--locked','--offline','--no-default-features','-p','fvid-media','--example','bench_mp4_uniform_plan_scan']
 with(q/'baseline-final-build.log').open('wb')as f:r=subprocess.run(cmd,stdout=f,stderr=subprocess.STDOUT,env=env)
 assert r.returncode==0
 src=Path(env['CARGO_TARGET_DIR'])/'release/examples/bench_mp4_uniform_plan_scan';dst=src.parent/'bench_mp4_uniform_plan_scan-baseline';shutil.copy2(src,dst);(q/'baseline-final-binary.json').write_text(json.dumps(dict(path=str(dst),sha256=hashlib.sha256(dst.read_bytes()).hexdigest(),source={n:hashlib.sha256(b).hexdigest()for n,b in published.items()}),indent=2)+'\n')
 cmd=['cargo','test','--locked','--offline','--no-default-features','--features','player','--test','mp4_uniform_plan_scan']
 with(q/'before-correctness.log').open('wb')as f:r=subprocess.run(cmd,stdout=f,stderr=subprocess.STDOUT,env=env)
 (q/'before-correctness-result.json').write_text(json.dumps(dict(command=cmd,exit_code=r.returncode),indent=2)+'\n')
finally:
 assert all(Path(n).read_bytes()==b for n,b in published.items()),'foreign changes;inspect preserved candidates'
 for n,b in candidate.items():Path(n).write_bytes(b)
print('Baseline terminal,candidate restored:',r.returncode,flush=True)
