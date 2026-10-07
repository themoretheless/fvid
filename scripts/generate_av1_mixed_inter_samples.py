#!/usr/bin/env python3
"""Owned mixed lossless/lossy inter-prediction fixtures; no private media input."""
import argparse,hashlib,json,subprocess
from pathlib import Path
from generate_av1_show_existing_samples import Bits,obu,sequence,webm
from generate_av1_mixed_lossless_samples import encode,key

def signed_subexp(b,low,high,reference,value):
    n=high-low;r=reference-low;x=value-low
    assert 0<=r<n and 0<=x<n
    if 2*r>n:r=n-1-r;x=n-1-x
    v=x if x>2*r else 2*(x-r) if x>=r else 2*(r-x)-1
    i=0;offset=0
    while True:
        bits=3 if i==0 else i+2;a=1<<bits
        if n<=offset+3*a:
            count=n-offset;value=v-offset;w=count.bit_length();m=(1<<w)-count
            if value<m:b.u(value,w-1)
            else:
                value+=m;b.u(value>>1,w-1);b.u(value&1)
            return
        more=v>=offset+a;b.u(int(more))
        if not more:b.u(v-offset,bits);return
        offset+=a;i+=1

def frame(entropy,base,selected,adaptive,reference,forced_reference=False,forced_tools=(0,0),global_models=None,previous_globals=None,interpolation=0,reference_select=False,segment_references=None,motion_switchable=False,size=None,refresh=128):
    b=Bits();b.u(0);b.u(1,2);b.u(1);b.u(0);b.u(int(not adaptive));b.u(int(size is not None));b.u(0,3);b.u(refresh,8)
    for logical in range(1,8):b.u(7 if logical==reference else 0,3)
    if size is not None:
        for _ in range(7):b.u(0)
        b.u(size[0]-1,5);b.u(size[1]-1,5)
    b.u(0);b.u(0);b.u(int(interpolation==4))
    if interpolation!=4:b.u(interpolation,2)
    b.u(int(motion_switchable))
    if adaptive:b.u(1)
    b.u(1);b.u(base,8);b.u(0,4);b.u(1);b.u(1);b.u(0);b.u(1)
    for seg in range(8):
        for feature in range(8):
            active=seg<2 and (feature==0 or forced_reference and feature==5 or feature==6 and forced_tools[seg]&1 or feature==7 and forced_tools[seg]&2);b.u(int(bool(active)))
            if active and feature<6:b.u((-base if seg==0 else 0) if feature==0 else (segment_references[seg] if segment_references is not None else reference),9 if feature==0 else 3)
    b.u(0);b.u(0,16);b.u(int(selected));b.u(int(reference_select));b.u(0)
    identity=[0,0,65536,0,0,65536]
    for logical in range(7):
        kind,params=(global_models or [(0,identity)]*7)[logical]
        old=(previous_globals or [identity]*7)[logical]
        b.u(int(kind!=0))
        if not kind:continue
        b.u(int(kind==2))
        if kind!=2:b.u(int(kind==1))
        indices=([2,3]+([4,5] if kind==3 else []) if kind>=2 else [])+[0,1]
        for index in indices:
            absolute,precision=(12,15) if index>=2 else (8,2) if kind==1 else (12,6)
            shift=16-precision;center=65536 if index%3==2 else 0;sub=1<<precision if center else 0
            maximum=1<<absolute;value=(params[index]-center)>>shift;reference=(old[index]>>shift)-sub
            signed_subexp(b,-maximum,maximum+1,reference,value)
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
