#!/usr/bin/env python3
"""Owned AV1 short-reference signaling streams with explicit-map twins."""
import argparse, hashlib, json, subprocess
from pathlib import Path
from generate_av1_show_existing_samples import Bits, obu, sequence, webm

def hidden(hint, bits, slot=None, frame_id=None):
    b=Bits();b.u(0);b.u(0 if slot is None else 2,2);b.u(0);b.u(1)
    b.u(int(slot is None));b.u(0)
    if frame_id is not None:b.u(frame_id,6)
    b.u(0);b.u(hint,bits)
    b.u(255 if slot is None else 1<<slot,8);b.u(0);b.u(0);b.u(1);b.u(0,8)
    b.u(0,6)
    return obu(6,b.bytes()+bytes([0x2e,0x03,0x80]))

def mapping(hints,current,bits,last,golden):
    # Independent sorted formulation of the normative set_frame_refs process.
    midpoint=1<<(bits-1);modulus=1<<bits
    order=[midpoint+((h-current+midpoint)%modulus-midpoint) for h in hints]
    result=[None]*7;result[0]=last;result[3]=golden;used={last,golden}
    backward=sorted((i for i in range(8) if i not in used and order[i]>=midpoint),key=lambda i:(order[i],i))
    if backward:result[6]=backward.pop();used.add(result[6])
    for target in [4,5]:
        if backward:result[target]=backward.pop(0);used.add(result[target])
    forward=sorted((i for i in range(8) if i not in used and order[i]<midpoint),key=lambda i:(order[i],i),reverse=True)
    for target in [1,2,4,5,6]:
        if result[target] is None and forward:result[target]=forward.pop(0)
    fallback=min(range(8),key=lambda i:(order[i],i))
    return [fallback if r is None else r for r in result]

def inter(current,bits,last,golden,refs=None,frame_id=None,expected_refs=None):
    b=Bits();b.u(0);b.u(1,2);b.u(1);b.u(0);b.u(1)
    if frame_id is not None:b.u(frame_id,6)
    b.u(0);b.u(current,bits)
    b.u(7,3);b.u(1,8);b.u(int(refs is None))
    if refs is None:b.u(last,3);b.u(golden,3)
    for r in (expected_refs if refs is None else refs):
        if refs is not None:b.u(r,3)
        if frame_id is not None:b.u(frame_id-(r+2)-1,4)
    b.u(0);b.u(1);b.u(0);b.u(0,2);b.u(0);b.u(1);b.u(0,8)
    b.u(0,5);b.u(0);b.u(0);b.u(0,7)
    return obu(6,b.bytes()+bytes([0x8c,0x70]))

def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--oracle',type=Path);args=parser.parse_args()
    root=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors';records=[]
    cases=[('past',[0,1,2,3,4,5,6,7],8,0,3),('future',[0,2,4,6,8,9,10,11],7,0,3),('ties',[0]*8,1,0,0),('future-ties',[0,0,2,2,2,2,2,2],1,0,1),('wrap',[14,15,0,1,2,3,4,5],2,0,3)]
    for name,hints,current,last,golden in cases:
        bits=4;prefix=sequence(False,False,False,order_bits=bits)+hidden(hints[0],bits)
        for slot,hint in enumerate(hints):prefix+=hidden(hint,bits,slot)
        refs=mapping(hints,current,bits,last,golden)
        for mode in ['short','explicit']:
            data=prefix+inter(current,bits,last,golden,None if mode=='short' else refs,expected_refs=refs)
            filename=f'av1-short-ref-{name}-{mode}.obu';(root/filename).write_bytes(data)
            records.append(dict(file=filename,sha256=hashlib.sha256(data).hexdigest(),references=refs,shown=1))
    for bits in range(1,9):
        modulus=1<<bits;midpoint=modulus>>1
        for ids in [False,True]:
            for pattern in ['past-ties','future-ties','mixed']:
                for last,golden in [(0,1),(7,6),(3,3)]:
                    current=0
                    hints=[modulus-1]*8 if pattern=='past-ties' else ([0]*8 if pattern=='future-ties' else [i%midpoint for i in range(8)])
                    hints[last]=modulus-1;hints[golden]=modulus-1
                    refs=mapping(hints,current,bits,last,golden)
                    prefix=sequence(ids,False,False,width=6,delta_bits=4,order_bits=bits)+hidden(hints[0],bits,frame_id=1 if ids else None)
                    for slot,hint in enumerate(hints):prefix+=hidden(hint,bits,slot,slot+2 if ids else None)
                    for mode in ['short','explicit']:
                        data=prefix+inter(current,bits,last,golden,None if mode=='short' else refs,10 if ids else None,refs)
                        filename=f'av1-short-ref-b{bits}-ids{int(ids)}-{pattern}-l{last}-g{golden}-{mode}.obu'
                        (root/filename).write_bytes(data);records.append(dict(file=filename,sha256=hashlib.sha256(data).hexdigest(),references=refs,shown=1))
    refusals=[]
    for which in ['last','golden']:
        hints=[0]*8;hints[1]=1;bits=4;current=1
        last,golden=(1,0) if which=='last' else (0,1)
        prefix=sequence(False,False,False,order_bits=bits)+hidden(0,bits)
        for slot,hint in enumerate(hints):prefix+=hidden(hint,bits,slot)
        data=prefix+inter(current,bits,last,golden,expected_refs=[0]*7)
        filename=f'av1-short-ref-invalid-{which}-not-forward.obu';(root/filename).write_bytes(data)
        refusals.append(dict(file=filename,sha256=hashlib.sha256(data).hexdigest(),error='AV1 short LAST/GOLDEN reference is not forward'))
    for r in records[:10]:
        filename=r['file'].replace('.obu','.webm');data=webm((root/r['file']).read_bytes());(root/filename).write_bytes(data)
        r['webm']=filename;r['webm_sha256']=hashlib.sha256(data).hexdigest()
    if args.oracle:
        for r in records:subprocess.run([str(args.oracle),str(root/r['file']),str(r['shown'])],check=True)
    (root/'av1-short-ref-generated.json').write_text(json.dumps(dict(fixtures=records,refusals=refusals),indent=2)+'\n')
if __name__=='__main__':main()
