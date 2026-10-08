#!/usr/bin/env python3
"""Original PS temporal oracles/video; explicit offline Fraction/Decimal only."""
import json,hashlib
from fractions import Fraction as F
from decimal import Decimal as D
from generate_aac_ps_mixing_oracles import matrix
from generate_aac_ps_history_fixtures import targets,zero,encode
from generate_aac_ps_fixtures import sbr
from generate_he_aac_packet_fixtures import DEST,packet,asc,video_fixture

def shape(values):return [[list(values[b*8+i*2:b*8+i*2+2]) for i in range(4)] for b in range(len(values)//8)]
def flatten(values):return [v for row in values for pair in row for v in pair]
def strings(values):return shape([str(v) for v in flatten(values)])
def interpolate(previous,ends,borders,slots):
    points={0:flatten(previous)}
    # Initial anchor is at zero; a transmitted border zero overrides it.
    points.update({n:flatten(m) for n,m in zip(borders,ends)})
    positions=sorted(points);output=[]
    for n in range(slots):
        before=max(p for p in positions if p<=n)
        after=next((p for p in positions if p>n),before)
        if before==after:values=points[before]
        else:
            weight=F(n-before,after-before)
            values=[(a*(weight.denominator-weight.numerator)+b*weight.numerator)/weight.denominator for a,b in zip(points[before],points[after])]
        output.append(shape(values))
    return output

def main():
    cases=[]
    for bands in [20,34]:
        for slots in [24,30,32]:
            patterns=[[],[0],[slots-1],[0,1,slots-1],[slots//3,slots*2//3],[0,slots//4,slots//2,slots*3//4]]
            previous=shape([F((i*7)%29-14,8) for i in range(bands*8)])
            for borders in patterns:
                ends=[shape([F((i*11+e*17)%37-18,8) for i in range(bands*8)]) for e in range(len(borders))]
                result=interpolate(previous,ends,borders,slots)
                # Exact rational values are saved as numerator/denominator
                # text; all numeric tests consume these saved results offline.
                conv=lambda m:shape([str(v) for v in flatten(m)])
                cases.append(dict(bands=bands,slots=slots,borders=borders,previous=conv(previous),endpoints=[conv(v) for v in ends],expected=[conv(v) for v in result]))
    blob=bytearray();videos=[]
    for mode in [1,5]:
        bands=[10,20,34][mode%3];previous_native=zero();previous=shape([D(0)]*(bands*8));frames=[];payloads=[];expected=[]
        for stage,borders in enumerate([[0,12,23],[8,24],[]]):
            native=[targets(mode,mode,stage*7+e,phase=False) for e in range(len(borders))]
            text,encoded=encode(stage==0,mode,mode,True,True,False,native,previous_native,32,variable=bool(borders),explicit_borders=borders if borders else None)
            assert encoded==borders
            raw=sbr(text,stage);data=packet(raw);frames.append(dict(offset=len(blob),bytes=len(data)));blob.extend(data);payloads.append(raw.hex())
            endpoints=[]
            grid=[-25,-18,-14,-10,-7,-4,-2,0,2,4,7,10,14,18,25] if mode<3 else [-50,-45,-40,-35,-30,-25,-22,-19,-16,-13,-10,-8,-6,-4,-2,0,2,4,6,8,10,13,16,19,22,25,30,35,40,45,50]
            icc=[D(v) for v in ['1','.937','.84118','.60092','.36764','0','-.589','-1']]
            for row in native:
                values=[]
                for iid,coherence in zip(row['iid'],row['icc']):
                    for h in matrix(str(grid[iid+len(grid)//2]),icc[coherence],'a' if mode<3 else 'b'):values.extend([h,D(0)])
                endpoints.append(shape(values))
            result=interpolate(previous,endpoints,borders,32)
            expected.append(dict(borders=borders,endpoints=[strings(m) for m in endpoints],coefficients=[strings(m) for m in result]))
            if native:previous_native=native[-1];previous=endpoints[-1]
        case=dict(slots=16,bands=64,frames=frames,asc=asc(24000,48000,16,'explicit',ps=True).hex(),pcm_offset=0,samples=12288)
        video=video_fixture([case],blob,channels=2,filename=f'he-aac-ps-interpolation-{bands}-synthetic.mp4')
        video.pop('pcm_offset');video.pop('samples');video['pcm_acceptance']='pending owned PS hybrid/decorrelation/synthesis'
        videos.append(dict(video=video,bands=bands,sbr_payloads=payloads,packet_frames=frames,expected=expected))
    (DEST/'he-aac-ps-interpolation-packets.bin').write_bytes(blob)
    (DEST/'aac-ps-interpolation-oracles.json').write_text(json.dumps(dict(kind='original Fraction/Decimal piecewise interpolation',cases=cases,videos=videos,packet_sha256=hashlib.sha256(blob).hexdigest()),separators=(',',':'))+'\n')
    print(len(cases),'numeric cases;',len(videos),'synthetic videos')
if __name__=='__main__':main()
