#!/usr/bin/env python3
"""Own AAC-LD spectra/PCM and AOT23 gap videos, offline direct cosine oracle."""
import json,math,struct
from generate_aac_main_tools_fixtures import channel,ics,f32
from generate_he_aac_packet_fixtures import DEST,field,frequency,packed,video_fixture
SHAPES=[0,0,1,1,0,1,0,0]*3

def window(n,i,shape):
    if not shape:return math.sin(math.pi*(i+.5)/(2*n))
    width=2*n
    if i<3*width//16 or i>=13*width//16:return 0.
    if 5*width//16<=i<11*width//16:return 1.
    origin=3*width/16 if i<n else 9*width/16
    return math.sin(math.pi*(i-origin+.5)/(width/4))

def main():
    packets=bytearray();gold=bytearray();analysis=bytearray();cases=[]
    for n in (480,512):
        rows=[];overlap=[0.]*n;previous=0;start=len(gold)
        for frame,shape in enumerate(SHAPES):
            q=[(-1 if (frame+k)%3==0 else 1 if (frame+k)%3==1 else 0) for k in range(8)]
            if frame in (8,9):q=[0]*8
            spectrum=[v*1024. for v in q]
            raw=packed('0000'+channel(0,[1,1],[q],info=ics(0,2,False,shape=shape)))
            row=dict(offset=len(packets),bytes=len(raw),shape=shape,spectrum=spectrum,reference_offset=len(gold));packets.extend(raw);rows.append(row)
            transformed=[2/n*sum(v*math.cos(math.pi/n*(i+.5+n/2)*(k+.5)) for k,v in enumerate(spectrum)) for i in range(2*n)]
            out=[f32((overlap[i]+transformed[i]*window(n,i,previous))/65536) for i in range(n)]
            overlap=[transformed[n+i]*window(n,n+i,shape) for i in range(n)];previous=shape
            gold.extend(struct.pack('<'+'f'*n,*out))
        asc=packed(field(23,5)+frequency(24000)+'0001'+field(n==480,1)+'00'+'00').hex()
        c=dict(name=str(n),n=n,asc=asc,channels=1,frames=rows,reference_offset=start,reference_bytes=len(gold)-start,container_rate=24000,container_frame_samples=n,samples=len(rows)*n,pcm_offset=0,slots=n//64,bands=32)
        c['video']=video_fixture([c],packets,filename=f'aac-ld-filterbank-{n}-synthetic.mp4');cases.append(c)
        c['analysis']=[]
        for before,current in ((0,0),(0,1),(1,0),(1,1)):
            values=[math.sin(.013*(i+1))+.4*math.cos(.021*i) for i in range(2*n)]
            ref=[sum(v*window(n,i,before if i<n else current)*math.cos(math.pi/n*(i+.5+n/2)*(k+.5)) for i,v in enumerate(values)) for k in range(n)]
            c['analysis'].append(dict(previous=before,current=current,reference_offset=len(analysis)));analysis.extend(struct.pack('<'+'d'*n,*ref))
    (DEST/'aac-ld-filterbank-packets.bin').write_bytes(packets);(DEST/'aac-ld-filterbank-reference.f32le').write_bytes(gold);(DEST/'aac-ld-filterbank-analysis.f64le').write_bytes(analysis)
    (DEST/'aac-ld-filterbank.json').write_text(json.dumps(dict(cases=cases,provenance='Own AOT23 ep0 no-prediction/no-resilience packets and sine/low-overlap direct cosine/window/overlap oracle. Public profile remains a gap until packet/LD-LTP integration. No private or foreign media, FFmpeg or network.'),indent=2)+'\n')
    print('generated two LD gap videos, 48 PCM blocks and eight forward-MDCT cases')
if __name__=='__main__':main()
