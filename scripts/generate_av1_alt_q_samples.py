#!/usr/bin/env python3
"""Owned AV1 ALT_Q fixtures with nonzero residuals and independent saved pixels."""
import argparse,hashlib,json,subprocess
from pathlib import Path
from generate_av1_show_existing_samples import Bits,obu,sequence,webm

def quant(b,base,features):
    b.u(base,8);b.u(0,4);b.u(int(features is not None))
    if features is not None:
        b.u(0);b.u(1) # unchanged inherited map, update feature data
        for segment in range(8):
            for feature in range(8):
                value=features.get(segment) if feature==0 else None;b.u(int(value is not None))
                if value is not None:b.u(value,9)
    if base:b.u(0) # no per-superblock delta Q
    coded_lossless=all(max(0,min(255,base+(features or {}).get(segment,0)))==0 for segment in range(8))
    if not coded_lossless:b.u(0,16) # loop filter levels/sharpness/deltas disabled
    return coded_lossless

def key(template):
    b=Bits();b.u(0);b.u(0,2);b.u(0);b.u(1);b.u(1);b.u(1);b.u(0);b.u(255,8);b.u(0);b.u(1)
    lossless=quant(b,template['qindex'],None)
    if not lossless:b.u(int(template['key_tx_mode']==2))
    b.u(0)
    return obu(6,b.bytes()+bytes(template['key_tile']))

def inter(template,base,delta):
    b=Bits();b.u(0);b.u(1,2);b.u(1);b.u(0);b.u(1);b.u(0);b.u(0,3);b.u(1,8)
    b.u(0,21);b.u(0);b.u(int(template["high_precision_mv"]));b.u(0);b.u(0,2);b.u(0);b.u(1)
    features={segment:delta for segment in range(8)} if template['qindex']==0 else {0:delta}
    lossless=quant(b,base,features)
    if not lossless:b.u(int(template['inter_tx_mode']==2))
    b.u(0);b.u(0);b.u(0,7)
    return obu(6,b.bytes()+bytes(template['inter_tile']))

def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--oracle',type=Path);args=parser.parse_args()
    root=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors'
    templates=json.loads((root/'av1-alt-q-owned-entropy.json').read_text())['templates'];records=[]
    for template in templates:
        target=template['qindex']
        cases=[(base,64-base) for base in [1,8,16,20,32,60,63,64,65,96,120,128,255]] if target==64 else ([(base,-255) for base in [1,4,32,64,128,255]] if target==0 else [(1,255),(64,191),(128,255),(200,255),(254,1),(255,255)])
        for base,delta in cases:
            filename=f'av1-alt-q-target{target}-base{base}-delta{delta}.obu';data=sequence(False,False,False)+key(template)+inter(template,base,delta);(root/filename).write_bytes(data)
            expected=filename.replace('.obu','.yuv')
            if args.oracle:subprocess.run([str(args.oracle),str(root/filename),'1',str(root/expected)],check=True)
            if not (root/expected).exists():raise SystemExit('Generate independent pixels with --oracle before saving manifest')
            reference=(root/expected).read_bytes();assert len(reference)==1536
            record=dict(file=filename,sha256=hashlib.sha256(data).hexdigest(),reference=expected,reference_sha256=hashlib.sha256(reference).hexdigest(),target=target,base=base,delta=delta)
            if base in [1,255]:
                name=filename.replace('.obu','.webm');wrapped=webm(data);(root/name).write_bytes(wrapped);record.update(webm=name,webm_sha256=hashlib.sha256(wrapped).hexdigest())
            records.append(record)
    (root/'av1-alt-q-generated.json').write_text(json.dumps(dict(fixtures=records),indent=2)+'\n')
if __name__=='__main__':main()
