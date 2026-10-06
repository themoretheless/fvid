#!/usr/bin/env python3
"""Owned flat AV1 inter-frame ID reference regressions; no runtime oracle required."""
import argparse, hashlib, json, subprocess
from pathlib import Path
from generate_av1_show_existing_samples import Bits, obu, sequence, hidden, show

def inter(current, slot, delta, bad_index=None, width=5, delta_bits=2):
    b=Bits(); b.u(0); b.u(1,2); b.u(1); b.u(1); b.u(1)
    b.u(current,width); b.u(0); b.u(1<<slot,8)
    for index in range(7):
        b.u(slot,3); b.u((delta-1 if index!=bad_index else delta)%(1<<delta_bits),delta_bits)
    b.u(0) # render size
    b.u(1); b.u(0); b.u(0,2); b.u(0) # high precision, fixed filter, simple motion
    b.u(1); b.u(0,8) # uniform tile, lossless quantization
    for _ in range(5): b.u(0) # deltas, matrix, segmentation
    b.u(0); b.u(0); b.u(0,7) # single reference, transforms, identity global motion
    # Owned flat inter entropy from scripts/av1_fixture.c with disabled order hints,
    # CDF updates and error-resilient frames. Every reconstructed sample is 128.
    return obu(6,b.bytes()+bytes([0x8c,0x70]))

def main():
    parser=argparse.ArgumentParser(description=__doc__); parser.add_argument('--oracle',type=Path); args=parser.parse_args()
    root=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors'; accepted=[]; refused=[]
    for name,old,current in [('next',7,8),('edge',7,11),('wrap',30,1)]:
        for slot in [0,7]:
            prefix=sequence(True,False,False)+hidden(True,False,False,old,0,slot)
            data=prefix+inter(current,slot,(current-old)%32)
            filename=f'av1-inter-id-{name}-slot{slot}.obu'; (root/filename).write_bytes(data)
            accepted.append(dict(file=filename,sha256=hashlib.sha256(data).hexdigest(),shown=1))
    for delta_bits in range(2,16):
        for width in range(delta_bits+1,min(delta_bits+8,16)+1):
            for mode in ['edge','wrap']:
                old,current=(1,1+(1<<delta_bits)) if mode=='edge' else ((1<<width)-2,1)
                prefix=sequence(True,False,False,width,delta_bits)+hidden(True,False,False,old,0,0,width)
                if mode=='edge':prefix+=hidden(True,False,False,2,2,7,width)
                data=prefix+inter(current,0,(current-old)%(1<<width),width=width,delta_bits=delta_bits)
                filename=f'av1-inter-id-d{delta_bits}-w{width}-{mode}.obu'; (root/filename).write_bytes(data)
                accepted.append(dict(file=filename,sha256=hashlib.sha256(data).hexdigest(),shown=1))
    for index in range(7):
        data=sequence(True,False,False)+hidden(True,False,False,7,0,0)+inter(8,0,1,index)
        filename=f'av1-inter-id-invalid-reference{index}.obu'; (root/filename).write_bytes(data)
        refused.append(dict(file=filename,sha256=hashlib.sha256(data).hexdigest(),error='AV1 inter reference frame ID mismatch'))
    data=sequence(True,False,False)+hidden(True,False,False,7,0,0)+hidden(True,False,False,11,2,0)+show(True,False,False,7,7)+inter(8,0,1)
    filename='av1-inter-id-shown-key-restore.obu'; (root/filename).write_bytes(data)
    accepted.append(dict(file=filename,sha256=hashlib.sha256(data).hexdigest(),shown=2))
    for name,current in [('repeat',7),('half-window',23),('backward',6)]:
        data=sequence(True,False,False)+hidden(True,False,False,7,0,0)+hidden(True,False,False,current,2,7)
        filename=f'av1-inter-id-invalid-current-{name}.obu'; (root/filename).write_bytes(data)
        refused.append(dict(file=filename,sha256=hashlib.sha256(data).hexdigest(),error='AV1 invalid current frame ID progression'))
    if args.oracle:
        for r in accepted: subprocess.run([str(args.oracle),str(root/r['file']),str(r['shown'])],check=True)
    (root/'av1-inter-id-generated.json').write_text(json.dumps(dict(fixtures=accepted,refusals=refused),indent=2)+'\n')
if __name__=='__main__': main()
