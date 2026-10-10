#!/usr/bin/env python3
"""Authored LD stereo/resilience packets and scalar PCM; offline, no FFmpeg."""
import json, math, struct
from generate_aac_main_tools_fixtures import f32
from generate_aac_ld_filterbank_fixtures import window
from generate_aac_ld_ltp_fixtures import GAINS
from generate_aac_er_ltp_resilient_fixtures import channel
from generate_he_aac_packet_fixtures import DEST, field, frequency, packed, video_fixture

class Oracle:
    def __init__(self,n,cos):
        self.n=n;self.cos=cos;self.timeline=[];self.overlap=[0.]*n;self.shape=0;self.lag=0
    def run(self,residual,shape,active,update,coefficient,used,reverse):
        n=self.n;lag=self.lag if update is None else update
        spectrum=residual[:];order=list(range(7,-1,-1) if reverse else range(8));a=math.sin(math.pi/7)
        if active:
            estimate=[]
            for i in range(2*n):
                relative=i-n-lag;absolute=len(self.timeline)+relative
                value=self.overlap[relative] if relative>=0 else self.timeline[absolute] if absolute>=0 else 0.
                estimate.append(value*GAINS[coefficient]*window(n,i,self.shape if i<n else shape))
            prediction=[sum(v*c for v,c in zip(estimate,self.cos[k])) for k in range(8)]
            prior=0.
            for k in order:
                original=prediction[k];prediction[k]=original+a*prior;prior=original
            for k in range(8):
                if used[k//4]:spectrum[k]=f32(spectrum[k]+prediction[k])
        prior=0.
        for k in order:
            value=spectrum[k]-a*prior;spectrum[k]=f32(value);prior=value
        transformed=[2/n*sum(spectrum[k]*self.cos[k][i] for k in range(8)) for i in range(2*n)]
        pcm=[self.overlap[i]+transformed[i]*window(n,i,self.shape) for i in range(n)]
        self.overlap=[transformed[n+i]*window(n,n+i,shape) for i in range(n)]
        self.timeline.extend(pcm);self.shape=shape
        if active:self.lag=lag
        return [f32(v/65536) for v in pcm]

def predictor(active,update,coefficient,used):
    return field(active,1)+(field(update is not None,1)+(field(update,10) if update is not None else '')+field(coefficient,3)+''.join(field(v,1) for v in used) if active else '')

def main():
    blob=bytearray();gold=bytearray();cases=[];malformed=[]
    for n in (480,512):
        cos=[[math.cos(math.pi/n*(i+.5+n/2)*(k+.5)) for i in range(2*n)] for k in range(8)]
        for flags in range(8):
            for mode in ('independent','common-0','common-1','common-2'):
                common=mode.startswith('common');ms=int(mode[-1]) if common else 0
                banks=[Oracle(n,cos),Oracle(n,cos)];rows=[];start=len(gold)
                for frame in range(12):
                    shapes=[(frame//2)%2,(frame//2)%2 if common else ((frame+1)//3)%2]
                    active=[frame>=1 and (frame+c)%4!=0 for c in range(2)]
                    updates=[(1023 if (frame+c)%3==0 else 13+71*c) if active[c] and frame in (1,3,6,9) else None for c in range(2)]
                    coefs=[(frame+3*c)%8 for c in range(2)];used=[[True,(frame+c)%3!=1] for c in range(2)]
                    pred=[predictor(active[c],updates[c],coefs[c],used[c]) for c in range(2)]
                    books=[[17 if flags&4 else 1,1] for _ in range(2)]
                    q=[[(frame+k+c)%3-1 for k in range(8)] for c in range(2)]
                    for c in range(2):
                        if flags&4:q[c][:4]=[v*17 for v in q[c][:4]]
                    scales=[[140+(-8,7,0,8)[(frame+c)%4],140] for c in range(2)]
                    reverse=[bool((frame+c)%2) for c in range(2)]
                    info=['000'+field(shapes[c],1)+field(2,6) for c in range(2)]
                    kwargs=[dict(flags=flags,scale_values=scales[c],tns=reverse[c]) for c in range(2)]
                    mutation,error=('zero-longest','AAC HCR nonempty region has zero longest codeword') if flags&1 else ('reverse-gain','AAC RVLC reverse gain mismatch') if flags&2 else ('virtual-lav','AAC virtual codebook magnitude exceeds section limit') if flags&4 else (None,'trailing bytes')
                    mask=[frame%2==0,frame%3==0] if ms==1 else [ms==2]*2
                    wire='0000'+field(common,1);badwire=None;present=any(active)
                    if common:
                        wire+=info[0]+field(present,1)+field(ms,2)+(''.join(field(v,1) for v in mask) if ms==1 else '')
                        wire+=pred[0] if present else ''
                        wire+=channel(0,books[0],[q[0]],**kwargs[0])
                        wire+=pred[1] if present else ''
                        prefix=wire
                        wire+=channel(0,books[1],[q[1]],**kwargs[1])
                        if frame==3:badwire=prefix+channel(0,books[1],[q[1]],mutation=mutation,**kwargs[1])
                    else:
                        for c in range(2):
                            header=info[c]+field(active[c],1)+(pred[c] if active[c] else '')
                            if frame==3 and c==1:badwire=wire+channel(0,books[c],[q[c]],info=header,mutation=mutation,**kwargs[c])
                            wire+=channel(0,books[c],[q[c]],info=header,**kwargs[c])
                    raw=packed(wire);row=dict(offset=len(blob),bytes=len(raw),reference_offset=len(gold));blob.extend(raw);rows.append(row)
                    residual=[[f32(math.copysign(abs(v)**(4/3)*2**((scales[c][k//4]-100)/4),v)) if v else 0. for k,v in enumerate(q[c])] for c in range(2)]
                    for k in range(8):
                        if common and mask[k//4]:
                            a,b=residual[0][k],residual[1][k];residual[0][k]=f32(a+b);residual[1][k]=f32(a-b)
                    pcm=[banks[c].run(residual[c],shapes[c],active[c],updates[c],coefs[c],used[c],reverse[c]) for c in range(2)]
                    for pair in zip(*pcm):gold.extend(struct.pack('<ff',*pair))
                    if badwire is not None:
                        bad=packed(badwire)+(b'\0' if flags==0 else b'')
                        row['malformed']=dict(offset=len(blob),bytes=len(bad),error=error);blob.extend(bad)
                asc=packed(field(23,5)+frequency(24000)+field(2,4)+field(n==480,1)+'01'+field(flags,3)+'000').hex()
                name=f'{n}-{flags}-{mode}'
                case=dict(name=name,n=n,flags=flags,mode=mode,channels=2,asc=asc,frames=rows,reference_offset=start,reference_bytes=len(gold)-start,container_rate=24000,container_frame_samples=n,samples=12*n,pcm_offset=0,slots=n//64,bands=32)
                case['video']=video_fixture([case],blob,channels=2,filename=f'aac-ld-stereo-{name}-synthetic.mp4');cases.append(case)
                bad=rows[3]['malformed'];badcase=dict(case,frames=rows[:3]+[bad])
                malformed.append(dict(video=video_fixture([badcase],blob,channels=2,filename=f'aac-ld-stereo-{name}-malformed-synthetic.mp4'),error=error))
    (DEST/'aac-ld-stereo-packets.bin').write_bytes(blob);(DEST/'aac-ld-stereo-reference.f32le').write_bytes(gold)
    (DEST/'aac-ld-stereo.json').write_text(json.dumps(dict(cases=cases,malformed=malformed,provenance='Own AOT23 stereo, all resilience flag combinations, virtual book17, RVLC escapes, HCR, independent/common windows, MS modes, deferred independent LD predictors and bidirectional TNS. Scalar absolute timeline/cosine/FIR/AR/LD-window PCM oracle. No private media, FFmpeg, network or foreign decoder.'),indent=2)+'\n')
    print(f'generated {len(cases)} LD stereo acceptance and {len(malformed)} malformed videos')
if __name__=='__main__':main()
