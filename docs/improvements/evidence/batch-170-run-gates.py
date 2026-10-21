import subprocess,os,hashlib,json
from pathlib import Path
root=Path.cwd();evidence=root/'docs/improvements/evidence'
sources=[root/'src/playback/audio/aiff.rs',root/'tests/pcm_run_alignment.rs',root/'scripts/generate_pcm_run_alignment_regressions.py']
hashes={str(p):hashlib.sha256(p.read_bytes()).hexdigest()for p in sources}
env=dict(os.environ,CARGO_INCREMENTAL='0',CARGO_TARGET_DIR='target');base=['cargo','test','--locked','--offline','--features','player']
checks=[('related-after',base+sum((['--test',n]for n in ['aifc_compression_type','aifc_float_native','audio_reader_eof','pcm_exact_seek','g711_native','audio_truncated_eof','block_audio_eof','audio_read_limit','audio_duplicate_chunks']),[])),('aiff-units',base+['--lib','playback_aiff']),('core',['cargo','test','--locked','--offline','--no-default-features','--test','aifc_compression_type']),('player-build',['cargo','build','--locked','--offline','--features','player'])]
results=[]
for name,command in checks:
    log=evidence/f'batch-170-{name}.log'
    with log.open('w')as f:r=subprocess.run(command,stdout=f,stderr=subprocess.STDOUT,env=env)
    results.append(dict(name=name,command=command,exit_code=r.returncode,log=str(log.relative_to(root))))
    (evidence/'batch-170-gate-results.json').write_text(json.dumps(dict(source_hashes=hashes,checks=results),indent=2)+'\n')
    print(name,r.returncode,flush=True);assert r.returncode==0,name
assert all(hashlib.sha256(Path(p).read_bytes()).hexdigest()==h for p,h in hashes.items())
