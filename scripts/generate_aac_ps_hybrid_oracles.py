#!/usr/bin/env python3
"""Offline owned hybrid-bank numerical references; explicit generator only.
Independent Decimal full convolution, no Rust output, FFmpeg or decoder.
"""
import json,struct
from decimal import Decimal as D
from pathlib import Path
from generate_aac_ps_hybrid_filter_oracles import PROTOTYPES
from generate_aac_ps_mixing_oracles import PI,sincos
ROOT=Path(__file__).resolve().parents[1]
DEST=ROOT/'tests/fixtures/playback-errors'
# Numeric topology from SP-040428 figures 8.3/8.5, in routed s_k order.
TOPOLOGY={20:[(0,6),(0,7),(0,0),(0,1),(0,2,5),(0,3,4),(1,1),(1,0),(2,0),(2,1)],
          34:[(p,q) for p,width in enumerate([12,8,4,4,4]) for q in range(width)]}
NAMES={20:['TwentyEight','TwentyTwo','TwentyTwo'],
       34:['ThirtyFourTwelve','ThirtyFourEight']+['ThirtyFourFour']*3}
def kernels():
    result={}
    for name,(width,half) in PROTOTYPES.items():
        taps=list(map(D,half));taps+=taps[-2::-1]
        groups=[]
        for q in range(width):
            channel=[]
            for tap,g in enumerate(taps):
                sn,cs=sincos(2*PI*(D(q)+(D(0) if name=='TwentyTwo' else D('.5')))*(tap-6)/width)
                channel.append((g*cs,D(0) if name=='TwentyTwo' else g*sn))
            groups.append(channel)
        result[name]=groups
    return result
KERNELS=kernels()
def signal(n,k,kind):
    if kind=='impulses':return D(int(n==k+1 and k<6)),D(int(n==8 and k in [0,1,2,3,4,63]))/2
    return D((13*n+7*k)%23-11)/16,D((3*n+11*k)%31-15)/32

def main():
    blob=bytearray()
    def save(slots):
        offset=len(blob);flat=[float(x) for slot in slots for pair in slot for x in pair]
        blob.extend(struct.pack('<'+'d'*len(flat),*flat));return [offset,len(flat)]
    cases=[]
    scenarios=[('twenty_complex',[(20,47)],'complex'),('thirty_four_complex',[(34,47)],'complex'),
               ('twenty_impulses',[(20,32)],'impulses'),('thirty_four_impulses',[(34,32)],'impulses'),
               ('grid_video',[(20,32),(34,32),(20,32)],'complex'),
               ('grid_30',[(34,30),(20,30),(34,30)],'complex')]
    for name,parts,kind in scenarios:
        total=sum(count for _,count in parts)
        input=[[signal(n,k,kind) for k in range(64)] for n in range(total)]
        frames=[];start=0
        for bands,count in parts:
            output=[]
            for n in range(start,start+count):
                raw=[]
                for p,proto in enumerate(NAMES[bands]):
                    raw_p=[]
                    for taps in KERNELS[proto]:
                        r=i=D(0)
                        # Direct full convolution in absolute input time.
                        for tap,(gr,gi) in enumerate(taps):
                            if n>=tap:
                                xr,xi=input[n-tap][p];r+=xr*gr-xi*gi;i+=xr*gi+xi*gr
                        raw_p.append((r,i))
                    raw.append(raw_p)
                routed=[]
                for p,*indices in TOPOLOGY[bands]:
                    routed.append(tuple(sum((raw[p][q][c] for q in indices),D(0)) for c in range(2)))
                first=len(NAMES[bands])
                routed.extend(input[n-6][k] if n>=6 else (D(0),D(0)) for k in range(first,64))
                output.append(routed)
            inverse=[]
            bindings=json.loads((DEST/'aac-ps-mapping-protocol.json').read_text())['hybrid'+str(bands)]
            for slot in output:
                row=[[D(0),D(0)] for _ in range(64)]
                for pair,binding in zip(slot,bindings):
                    k=binding['qmf']
                    for c in range(2):row[k][c]+=pair[c]
                inverse.append(row)
            frames.append(dict(bands=bands,start=start,slots=count,output=save(output),synthesis=save(inverse)))
            start+=count
        cases.append(dict(name=name,kind=kind,input=save(input),frames=frames))
    cases[4]['video']='he-aac-ps-matrix-grid-retain-synthetic.mp4'
    manifest=dict(source='GOST R 53556.8-2013 6.4.3/6.4.7; SP-040428 figures 8.3/8.5; own Decimal convolution',topology=TOPOLOGY,cases=cases)
    (DEST/'aac-ps-hybrid-oracles.json').write_text(json.dumps(manifest,indent=2)+'\n')
    (DEST/'aac-ps-hybrid-reference.bin').write_bytes(blob)
if __name__=='__main__':main()
