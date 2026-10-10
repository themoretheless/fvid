#!/usr/bin/env python3
"""Own ER AAC-LTP baseline bitstreams, offline scalar prediction and PCM."""
import json, struct
from generate_aac_ltp_transition_fixtures import Oracle, SEQUENCES
from generate_aac_main_tools_fixtures import channel, ics
from generate_he_aac_packet_fixtures import DEST, field, frequency, packed, video_fixture

def prediction(active,lag,coef,used):
    return str(int(active))+(field(lag,11)+field(coef,3)+''.join(str(int(x)) for x in used) if active else '')

def main():
    blob=bytearray();gold=bytearray();cases=[];malformed=[]
    for n in (960,1024):
        for mode in ('mono','independent','common-0','common-1','common-2'):
            width=1 if mode=='mono' else 2;common=mode.startswith('common');ms=int(mode[-1]) if common else 0
            banks=[Oracle(n) for _ in range(width)];rows=[];start=len(gold)
            for frame,seq in enumerate(SEQUENCES):
                shape=frame%2;header=ics(seq,2,False,shape=shape);pred=[];values=[];active=[];used=[];lags=[];coefs=[]
                for c in range(width):
                    enabled=seq!=2 and frame>=1 and (frame+c)%4!=0
                    lag=n-13*((frame+c)%3);coef=(frame+3*c)%8;bands=[True,(frame+c)%3!=1]
                    active.append(enabled);lags.append(lag);coefs.append(coef);used.append(bands)
                    pred.append(prediction(enabled,lag,coef,bands))
                    values.append([[(-1 if (frame+w+k+c)%3==0 else 1 if (frame+w+k+c)%3==1 else 0) for k in range(8)] for w in range(8 if seq==2 else 1)])
                present=seq!=2 and any(active)
                wire='0000'
                if width==2:wire+=str(int(common))
                lag_at=None
                if common:
                    wire+=header if seq==2 else header[:-1]+str(int(present))
                    wire+=field(ms,2)+('10' if ms==1 else '')
                    if present:
                        if active[0]:lag_at=len(wire)+1
                        wire+=pred[0]
                    wire+=channel(seq,[1,1],values[0])
                    if present:wire+=pred[1]
                    wire+=channel(seq,[1,1],values[1])
                else:
                    for c in range(width):
                        info=header if seq==2 else header[:-1]+str(int(active[c]))+(pred[c] if active[c] else '')
                        if active[c] and lag_at is None:lag_at=len(wire)+8+len(header)+1
                        wire+=channel(seq,[1,1],values[c],info=info)
                raw=packed(wire);row=dict(offset=len(blob),bytes=len(raw),sequence=seq,active=active,reference_offset=len(gold));blob.extend(raw);rows.append(row)
                mixed=[[r[:] for r in q] for q in values]
                if common:
                    for w in range(len(mixed[0])):
                        for k in range(8):
                            if ms==2 or (ms==1 and k<4):
                                a,b=mixed[0][w][k],mixed[1][w][k];mixed[0][w][k]=a+b;mixed[1][w][k]=a-b
                pcm=[banks[c].run(seq,shape,mixed[c],active[c],lags[c],coefs[c],used[c]) for c in range(width)]
                for i in range(n):gold.extend(struct.pack('<'+'f'*width,*(p[i] for p in pcm)))
                if frame==2:
                    assert lag_at is not None
                    bad=packed(wire[:lag_at]+field(2*n+1,11)+wire[lag_at+11:]) if n==960 else raw[:-1]
                    row['malformed']=dict(offset=len(blob),bytes=len(bad),error='AAC LTP lag exceeds two frames' if n==960 else 'truncated or oversized bit field');blob.extend(bad)
            asc=packed(field(19,5)+frequency(24000)+field(width,4)+field(n==960,1)+'00'+'00')
            name=f'{n}-{mode}';case=dict(name=name,n=n,asc=asc.hex(),channels=width,frames=rows,reference_offset=start,reference_bytes=len(gold)-start,container_rate=24000,container_frame_samples=n,samples=12*n,pcm_offset=0,slots=n//64,bands=32)
            case['video']=video_fixture([case],blob,channels=width,filename=f'aac-er-ltp-{name}-synthetic.mp4');cases.append(case)
            bad=rows[2]['malformed'];badcase=dict(case,frames=rows[:2]+[bad]);malformed.append(dict(video=video_fixture([badcase],blob,channels=width,filename=f'aac-er-ltp-{name}-malformed-synthetic.mp4'),error=bad['error']))
    for case in list(cases):
        ext=dict(case,name=case['name']+'-extension',asc=packed(field(19,5)+frequency(24000)+field(case['channels'],4)+field(case['n']==960,1)+'01'+'0000'+'00').hex())
        ext['video']=video_fixture([ext],blob,channels=ext['channels'],filename=f"aac-er-ltp-{ext['name']}-synthetic.mp4");cases.append(ext)
    (DEST/'aac-er-ltp-packets.bin').write_bytes(blob);(DEST/'aac-er-ltp-reference.f32le').write_bytes(gold)
    (DEST/'aac-er-ltp.json').write_text(json.dumps(dict(cases=cases,malformed=malformed,provenance='Own AOT19 ep0 baseline packets, deferred common-window predictor data and scalar cosine/window/float-history oracle. No private media or foreign decoder.'),indent=2)+'\n')
    print(f'generated {len(cases)} acceptance and {len(malformed)} malformed videos')
if __name__=='__main__':main()
