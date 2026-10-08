#!/usr/bin/env python3
"""Generate owned distinct-plane matrix fixtures separately from offline tests.
Build av1_quant_matrix_plane_fixture.c with libaom and the native optional
av1_quant_matrix_planes_fixture example. References are generation-only.
"""
import argparse, hashlib, itertools, json, subprocess, tempfile
from pathlib import Path
from generate_av1_show_existing_samples import webm

def main():
    p=argparse.ArgumentParser(description=__doc__)
    for arg in ['encoder','rewriter','oracle','second-oracle']:
        p.add_argument('--'+arg,type=Path,required=True)
    a=p.parse_args(); root=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors'
    records=[]
    for depth,levels in itertools.product([8,10,12],[(0,7,14),(14,0,7),(7,14,0)]):
        name=f'av1-quant-matrix-planes-depth{depth}-qm'+ '-'.join(map(str,levels))
        stream=root/(name+'.obu'); reference=root/(name+'.yuv'); wrapped=root/(name+'.webm')
        with tempfile.TemporaryDirectory(prefix='fvid-owned-qm-planes-') as tmp:
            baseline=Path(tmp)/'baseline.obu'
            with baseline.open('wb') as out:
                subprocess.run([str(a.encoder),'64','1','32','1','1',str(depth),'0','0','0','0'],stdout=out,check=True)
            subprocess.run([str(a.rewriter),str(baseline),str(stream),*map(str,levels)],check=True)
            subprocess.run([str(a.oracle),str(stream),'1',str(reference),'whole-packet'],check=True)
            cross=Path(tmp)/'cross.yuv'
            subprocess.run([str(a.second_oracle),str(stream),str(cross)],check=True)
            assert cross.read_bytes()==reference.read_bytes(),name
        wrapped.write_bytes(webm(stream.read_bytes(),(64,64)))
        record=dict(file=stream.name,reference=reference.name,webm=wrapped.name,depth=depth,matrix=list(levels),size=[64,64],oracles=['libaom','dav1d'],generation='owned baseline; matrix levels plus required separate-UV flag/alignment; all other parsed header fields identical')
        for key,path in [('sha256',stream),('reference_sha256',reference),('webm_sha256',wrapped)]:
            record[key]=hashlib.sha256(path.read_bytes()).hexdigest()
        records.append(record)
    (root/'av1-quant-matrix-planes-generated.json').write_text(json.dumps(dict(fixtures=records),indent=2)+'\n')
if __name__=='__main__':main()
