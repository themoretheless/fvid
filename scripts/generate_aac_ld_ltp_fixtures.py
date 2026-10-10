#!/usr/bin/env python3
"""Authored LD-LTP/TNS packets, video reproducers and independent scalar PCM."""
import json, math, struct
from generate_aac_main_tools_fixtures import channel, f32
from generate_aac_ld_filterbank_fixtures import window
from generate_he_aac_packet_fixtures import DEST, field, frequency, packed, video_fixture

GAINS=[.570829,.696616,.813004,.911304,.984900,1.067894,1.194601,1.369533]

def main():
    blob=bytearray(); gold=bytearray(); cases=[]
    for n in (480,512):
        for reverse in (False,True):
            timeline=[]; overlap=[0.]*n; previous_shape=0; previous_lag=0; rows=[]
            cosine=[[math.cos(math.pi/n*(i+.5+n/2)*(k+.5)) for i in range(2*n)] for k in range(8)]
            for frame in range(16):
                shape=(frame//2)%2
                active=frame not in (0,5,12)
                update=active and frame in (1,3,7,10,14)
                lag={1:0,3:1023,7:n,10:13,14:1023}.get(frame,previous_lag)
                coefficient=frame%8; used=[frame%3!=0,frame%3!=1]
                q=[(frame+k)%3-1 for k in range(8)]
                if frame in (11,12):q=[0]*8
                info='000'+field(shape,1)+field(2,6)+field(active,1)
                if active:
                    info+='1'+field(update,1)+(field(lag,10) if update else '')+field(coefficient,3)+''.join(field(v,1) for v in used)
                raw=packed('0000'+channel(0,[1,1],[q],info=info,tns=(reverse,1),er=True))
                rows.append(dict(offset=len(blob),bytes=len(raw),shape=shape,active=active,lag_update=lag if update else None,coefficient=coefficient,used=used,spectrum=[v*1024. for v in q],reference_offset=len(gold)))
                blob.extend(raw)
                spectrum=[v*1024. for v in q]
                lpc=math.sin(math.pi/7)
                order=list(range(7,-1,-1) if reverse else range(8))
                if active:
                    estimate=[]
                    for i in range(2*n):
                        relative=i-n-lag
                        absolute=len(timeline)+relative
                        value=overlap[relative] if relative>=0 else timeline[absolute] if absolute>=0 else 0.
                        estimate.append(value*GAINS[coefficient]*window(n,i,previous_shape if i<n else shape))
                    predicted=[sum(v*c for v,c in zip(estimate,cosine[k])) for k in range(8)]
                    history=0.
                    for k in order:
                        original=predicted[k];predicted[k]=original+lpc*history;history=original
                    for k in range(8):
                        if used[k//4]:spectrum[k]=f32(spectrum[k]+predicted[k])
                history=0.
                for k in order:
                    value=spectrum[k]-lpc*history;spectrum[k]=f32(value);history=value
                transformed=[2/n*sum(spectrum[k]*cosine[k][i] for k in range(8)) for i in range(2*n)]
                pcm=[overlap[i]+transformed[i]*window(n,i,previous_shape) for i in range(n)]
                overlap=[transformed[n+i]*window(n,n+i,shape) for i in range(n)]
                timeline.extend(pcm);previous_shape=shape
                if update:previous_lag=lag
                gold.extend(struct.pack('<'+'f'*n,*[f32(v/65536) for v in pcm]))
            asc=packed(field(23,5)+frequency(24000)+'0001'+field(n==480,1)+'00'+'00').hex()
            c=dict(name=f'{n}-{int(reverse)}',n=n,reverse=reverse,asc=asc,channels=1,frames=rows,container_rate=24000,container_frame_samples=n,samples=16*n,pcm_offset=0,slots=n//64,bands=32)
            c['video']=video_fixture([c],blob,filename=f'aac-ld-ltp-{n}-tns-{int(reverse)}-synthetic.mp4');cases.append(c)
    (DEST/'aac-ld-ltp-packets.bin').write_bytes(blob)
    (DEST/'aac-ld-ltp-reference.f32le').write_bytes(gold)
    (DEST/'aac-ld-ltp.json').write_text(json.dumps(dict(cases=cases,provenance='Own AOT23 ep0 LTP lag updates/reuse/absence, sine/low-overlap switches and forward/reverse order-one TNS. Independent scalar cosine, absolute PCM timeline, FIR/AR and overlap oracle. Public mono AOT23 ep0 MP4 PCM is accepted; ER TNS follows gain-control presence. No private media, foreign codec, FFmpeg or network.'),indent=2)+'\n')
    print('generated four authored LD LTP/TNS videos and 64 scalar PCM frames')
if __name__=='__main__':main()
