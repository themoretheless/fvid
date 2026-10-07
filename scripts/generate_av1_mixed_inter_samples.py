#!/usr/bin/env python3
"""Owned mixed lossless/lossy inter-prediction fixtures; no private media input."""
import argparse,hashlib,json,subprocess
from pathlib import Path
from generate_av1_show_existing_samples import Bits,obu,sequence,webm
from generate_av1_mixed_lossless_samples import encode,key

def frame(entropy,base,selected,adaptive,reference):
    b=Bits();b.u(0);b.u(1,2);b.u(1);b.u(0);b.u(int(not adaptive));b.u(0);b.u(0,3);b.u(128,8)
    for logical in range(1,8):b.u(7 if logical==reference else 0,3)
    b.u(0);b.u(0);b.u(0);b.u(0,2);b.u(0)
    if adaptive:b.u(1)
    b.u(1);b.u(base,8);b.u(0,4);b.u(1);b.u(1);b.u(0);b.u(1)
    for seg in range(8):
        for feature in range(8):
            active=seg<2 and feature==0;b.u(int(active))
            if active:b.u(-base if seg==0 else 0,9)
    b.u(0);b.u(0,16);b.u(int(selected));b.u(0);b.u(0);b.u(0,7)
    return obu(6,b.bytes()+entropy)

def main():
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('--writer',type=Path,required=True);p.add_argument('--oracle',type=Path,required=True);a=p.parse_args();root=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors';records=[]
    for reference in range(1,8):
        for base in [1,64,255]:
            for mask in [1,6,9,14]:
                for selected in [False,True]:
                    for adaptive in [False,True]:
                        for residual in [1,-1]:
                            prefix,initial=encode(a.writer,base,mask,False,False,residual)
                            other,other_map=encode(a.writer,base,mask,False,False,-residual)
                            data=sequence(False,False,False)+key(prefix,base,False,False)+key(other,base,False,False,kind=2,refresh=128);maps=[initial,other_map]
                            for current in [mask,15-mask]:
                                entropy,grid=encode(a.writer,base,current,selected,adaptive,residual,inter=True,reference=reference)
                                data+=frame(entropy,base,selected,adaptive,reference);maps.append(grid)
                            name=f'av1-mixed-inter-ref{reference}-q{base}-mask{mask}-tx{int(selected)}-adapt{int(adaptive)}-dc{residual}';file=name+'.obu';expected=name+'.yuv';wrapped=name+'.webm'
                            (root/file).write_bytes(data);w=webm(data);(root/wrapped).write_bytes(w)
                            subprocess.run([str(a.oracle),str(root/file),'2',str(root/expected)],check=True)
                            pixels=(root/expected).read_bytes();assert len(pixels)==3072
                            records.append(dict(file=file,sha256=hashlib.sha256(data).hexdigest(),reference=expected,reference_sha256=hashlib.sha256(pixels).hexdigest(),webm=wrapped,webm_sha256=hashlib.sha256(w).hexdigest(),logical_reference=reference,physical_references=[7 if logical==reference else 0 for logical in range(1,8)],base=base,mask=mask,selected=selected,adaptive=adaptive,residual=residual,maps=maps))
    (root/'av1-mixed-inter-generated.json').write_text(json.dumps(dict(fixtures=records),indent=2)+'\n')
if __name__=='__main__':main()
