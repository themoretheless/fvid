import subprocess,os,hashlib,json
from pathlib import Path
root=Path.cwd(); evidence=root/'docs/improvements/evidence'
sources=[root/'src/container/avi.rs',root/'tests/avi_chunk_extent.rs']
before={str(p):hashlib.sha256(p.read_bytes()).hexdigest() for p in sources}
env=dict(os.environ,CARGO_INCREMENTAL='0',CARGO_TARGET_DIR='target')
base=['cargo','test','--locked','--offline','--features','player']
checks=[('related-after',base+sum((['--test',x] for x in ['avi_pcm','avi_adpcm_grouped','avi_riff_boundary','avi_index_origin','avi_main_header_bounds','avi_audio_eof','avi_duration_grid','audio_read_limit','g711_native']),[])),('root-units',base+['--lib','avi']),('core',['cargo','test','--locked','--offline','--no-default-features']+sum((['--test',x]for x in ['avi_chunk_extent','avi_pcm','avi_adpcm_grouped','avi_riff_boundary','avi_index_origin','avi_main_header_bounds']),[])),('player-build',['cargo','build','--locked','--offline','--features','player'])]
results=[json.loads((evidence/'batch-168-gate-results-initial.json').read_text())['checks'][0]]
assert results[0]['name']=='related-after' and results[0]['exit_code']==0
checks=checks[1:]
for name,command in checks:
    log=evidence/f'batch-168-{name}.log'
    with log.open('w') as f:r=subprocess.run(command,stdout=f,stderr=subprocess.STDOUT,env=env)
    results.append(dict(name=name,command=command,exit_code=r.returncode,log=str(log.relative_to(root))))
    (evidence/'batch-168-gate-results.json').write_text(json.dumps(dict(source_hashes=before,checks=results),indent=2)+'\n')
    print(name,r.returncode,flush=True)
    assert r.returncode==0,name
assert all(hashlib.sha256(Path(p).read_bytes()).hexdigest()==h for p,h in before.items())
