#!/usr/bin/env python3
"""Owned active AV1 loop-filter segmentation streams and saved oracle pixels."""
import argparse,hashlib,json,subprocess
from pathlib import Path
from generate_av1_show_existing_samples import Bits,obu,sequence,webm
from generate_av1_alt_q_samples import key

def inter(template,levels,features,deltas):
    b=Bits();b.u(0);b.u(1,2);b.u(1);b.u(0);b.u(1);b.u(0);b.u(0,3);b.u(1,8)
    b.u(0,21);b.u(0);b.u(int(template['high_precision_mv']));b.u(0);b.u(0,2);b.u(0);b.u(1)
    b.u(64,8);b.u(0,4);b.u(1);b.u(0);b.u(1)
    for segment in range(8):
        for feature in range(8):
            value=features.get(feature) if segment==0 else None;b.u(int(value is not None))
            if value is not None:b.u(value,7)
    b.u(0) # no delta Q
    b.u(levels[0],6);b.u(levels[1],6)
    if levels[0] or levels[1]:b.u(levels[2],6);b.u(levels[3],6)
    b.u(0,3);b.u(int(deltas))
    if deltas:
        b.u(1)
        for value in [1,2,0,0,-1,0,-1,-1,0,3]:b.u(1);b.u(value,7)
    b.u(int(template['inter_tx_mode']==2));b.u(0);b.u(0);b.u(0,7)
    return obu(6,b.bytes()+bytes(template['inter_tile']))

def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--oracle',type=Path);args=parser.parse_args()
    root=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors'
    template=next(t for t in json.loads((root/'av1-alt-q-owned-entropy.json').read_text())['templates'] if t['qindex']==64);records=[]
    cases=[]
    for feature in range(1,5):
        for delta in [-64,-63,-16,-1,0,1,16,31,63]:cases.append((f'f{feature}-d{delta}',[16,16,16,16],{feature:delta}))
    cases += [('mixed',[16,24,8,32],{1:31,2:-16,3:63,4:-63}),('vertical-zero',[0,16,8,8],{1:32}),('chroma-zero',[16,16,0,0],{3:63,4:63}),('all-zero',[0,0,0,0],{1:63,2:63,3:63,4:63}),('shift-cross',[31,31,31,31],{1:1,2:1,3:1,4:1})]
    for name,levels,features in cases:
        for deltas in [False,True]:
            filename=f'av1-alt-lf-{name}-refmode{int(deltas)}.obu';data=sequence(False,False,False)+key(template)+inter(template,levels,features,deltas);(root/filename).write_bytes(data)
            expected=filename.replace('.obu','.yuv')
            if args.oracle:subprocess.run([str(args.oracle),str(root/filename),'1',str(root/expected)],check=True)
            if not (root/expected).exists():raise SystemExit('Save independent pixels using --oracle')
            reference=(root/expected).read_bytes();assert len(reference)==1536
            r=dict(file=filename,sha256=hashlib.sha256(data).hexdigest(),reference=expected,reference_sha256=hashlib.sha256(reference).hexdigest(),levels=levels,features=features,refmode=deltas)
            if name in ['mixed','vertical-zero','chroma-zero','all-zero','shift-cross']:
                webm_name=filename.replace('.obu','.webm');wrapped=webm(data);(root/webm_name).write_bytes(wrapped);r.update(webm=webm_name,webm_sha256=hashlib.sha256(wrapped).hexdigest())
            records.append(r)
    (root/'av1-alt-lf-generated.json').write_text(json.dumps(dict(fixtures=records),indent=2)+'\n')
if __name__=='__main__':main()
