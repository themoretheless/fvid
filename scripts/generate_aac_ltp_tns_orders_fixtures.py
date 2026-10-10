#!/usr/bin/env python3
"""Authored LTP/PNS long TNS orders 1..12, scalar source/target histories."""
import json, math, struct
from generate_aac_ltp_pns_fixtures import Oracle, prediction
from generate_aac_main_tools_fixtures import channel as base_channel, Noise, f32, sc
from generate_aac_ld_ltp_fixtures import GAINS
from generate_he_aac_packet_fixtures import DEST, field, frequency, packed, video_fixture



def channel(seq,books,values,info='',tns=None):
    raw=base_channel(seq,books,values,info=info)
    if tns is None:return raw
    # 48-kHz has more than 47 long bands. Length63 reaches band zero.
    scale_bits=sum(9 if book==13 else len(sc(0)) for book in books if book!=0)
    at=8+len(info)+9*len(books)+scale_bits+1
    assert raw[at:at+2]=='00'
    reverse,order=tns[:2]
    resolution,compression=tns[2:] if len(tns)>2 else (3,0)
    coefficients=[-1 if (j+order)%3==0 else 1 for j in range(order)] if len(tns)>2 else [0]*(order-1)+[1]
    width=resolution-compression
    data='1'+field(1,2)+field(resolution-3,1)+field(63,6)+field(order,5)+field(reverse,1)+field(compression,1)+''.join(field(k%(1<<width),width) for k in coefficients)
    return raw[:at]+data+raw[at+1:]


def filter_values(values, reverse, order, resolution=None, compression=0, analysis=False):
    result=values[:];history=[]
    if resolution is None:lpc=[0.]*(order-1)+[math.sin(math.pi/7)]
    else:
        lpc=[]
        for j in range(order):
            k=-1 if (j+order)%3==0 else 1
            reflection=math.sin(k*math.pi/2/((1<<(resolution-1))+(.5 if k<0 else -.5)))
            lpc=[a+reflection*b for a,b in zip(lpc,reversed(lpc))]+[reflection]
    for k in (range(len(values)-1,-1,-1) if reverse else range(len(values))):
        previous=sum(lpc[j]*history[-1-j] for j in range(min(order,len(history))))
        value=values[k]+previous if analysis else values[k]-previous
        result[k]=value if analysis else f32(value)
        history.append(values[k] if analysis else value)
    return result


class LongOracle(Oracle):
    def run(self,residual,active,lag,coef,used,tns=None):
        n=self.n;values=residual[:];count=len(values)
        if active:
            estimate=[]
            for i in range(2*n):
                relative=i-lag;absolute=len(self.pcm)+relative
                sample=self.overlap[relative] if 0<=relative<n else self.pcm[absolute] if relative<0 and absolute>=0 else 0.
                estimate.append(sample*GAINS[coef]*self.weight(i,0))
            predicted=[sum(v*c for v,c in zip(estimate,self.cos[k])) for k in range(count)]
            if tns:predicted=filter_values(predicted,*tns,analysis=True)
            values=[f32(v+predicted[k]) if used[k//4] else v for k,v in enumerate(values)]
        if tns:values=filter_values(values,*tns)
        self.spectrum=values[:]
        transformed=[2/n*sum(values[k]*self.cos[k][i] for k in range(count)) for i in range(2*n)]
        out=[self.overlap[i]+transformed[i]*self.weight(i,0) for i in range(n)]
        self.overlap=[transformed[n+i]*self.weight(n+i,0) for i in range(n)]
        self.pcm.extend(out)
        return [f32(v/65536) for v in out]


def main(multitap=False):
    prefix='aac-ltp-tns-multitap' if multitap else 'aac-ltp-tns-orders'
    frames=48 if multitap else 24
    blob=bytearray();gold=bytearray();wrong=bytearray();cases=[];refusals=[]
    for n in (960,1024):
        cos=[[math.cos(math.pi/n*(i+.5+n/2)*(k+.5)) for i in range(2*n)] for k in range(24)]
        for point in (0,1):
            source=LongOracle(n,False,cos);bad=LongOracle(n,False,cos)
            targets=[LongOracle(n,False,cos) for _ in range(2)];bad_targets=[LongOracle(n,False,cos) for _ in range(2)]
            noise=Noise();rows=[];controls=[];start=len(gold)
            for frame in range(frames):
                active=frame>=1;order=frame%12+1;reverse=bool((frame+(frame//12 if multitap else 0))%2)
                variant=(frame//12)%4
                suffix=(3+variant//2,variant%2) if multitap else ()
                source_tns=(reverse,order)+suffix;target_tns=[(not reverse,13-order)+suffix,(reverse,order)+suffix]
                books=[1]*6
                if 3<=frame<=18:books[1]=13
                q=[(frame+k)%3-1 for k in range(24)];target_q=[[(frame+k+ch+1)%3-1 for k in range(24)] for ch in range(2)]
                lag=n-13;coef=frame%8;used=[b!=13 for b in books]
                changes=[3,2,-1,0,2,-3];total=0;gains=[]
                for delta in changes:
                    total+=delta;gains.append((-1. if total&1 else 1.)*2**(-(total>>1)*[.125,.25,.5,1.][frame%4]))
                base='0000'+field(6,6)
                def encode(flags):
                    info=base+field(active,1)+(prediction(4,True,lag,coef,flags) if active else '')
                    body=channel(0,books,[q],info=info,tns=source_tns)
                    target='0010000'+'1'+base+'0'+'00'+''.join(channel(0,[1]*6,[target_q[ch]],tns=target_tns[ch]) for ch in range(2))
                    cce='0100001'+'0'+'000'+'1'+'0000'+'11'+field(point==1,1)+'1'+field(frame%4,2)+body+'0'+''.join(sc(d) for d in changes)
                    return packed((cce+target if frame%2 else target+cce)+'111')
                for flags,listing in (([True]*6,rows),(used,controls)):
                    raw=encode(flags);listing.append(dict(offset=len(blob),bytes=len(raw),order=order,**(dict(resolution=suffix[0],compression=suffix[1],reverse=reverse) if multitap else {})));blob.extend(raw)
                spectrum=[float(v*1024) for v in q]
                if books[1]==13:spectrum[4:8]=noise.band(50)
                source.run(spectrum,active,lag,coef,used,source_tns)
                bad.run(spectrum,active,lag,coef,[True]*6,source_tns)
                def output(banks,values):
                    pcm=[]
                    for ch in range(2):
                        residual=[float(v*1024) for v in target_q[ch]]
                        if point==1:residual=filter_values(residual,*target_tns[ch])
                        mixed=[f32(v+f32(values[k]*(gains[k//4] if ch else 1.))) for k,v in enumerate(residual)]
                        pcm.append(banks[ch].run(mixed,False,0,0,[False]*6,target_tns[ch] if point==0 else None))
                    return b''.join(struct.pack('<ff',*pair) for pair in zip(*pcm))
                gold.extend(output(targets,source.spectrum));wrong.extend(output(bad_targets,bad.spectrum))
            asc_prefix=field(4,5)+frequency(48000)+'0000'+field(n==960,1)+'00'
            pce=field(0,4)+field(3,2)+frequency(48000)+field(1,4)+field(0,4)+field(0,4)+field(0,2)+field(0,3)+field(1,4)+'000'+'1'+field(0,4)+'0'+field(1,4)
            asc=packed(asc_prefix+pce+'0'*(-len(asc_prefix+pce)%8)+field(0,8)).hex()
            name=f'{n}-{point}';c=dict(name=name,n=n,point=point,asc=asc,channels=2,frames=rows,control_frames=controls,reference_offset=start,reference_bytes=len(gold)-start,container_rate=48000,container_frame_samples=n,samples=frames*n,slots=n//64,bands=32,pcm_offset=0)
            c['video']=video_fixture([c],blob,channels=2,filename=f'{prefix}-{name}-synthetic.mp4')
            c['control_video']=video_fixture([dict(c,frames=controls)],blob,channels=2,filename=f'{prefix}-{name}-control-synthetic.mp4');cases.append(c)
            if point==0:
                for rejected_order in (13,20):
                    body=channel(0,[1]*6,[[1,-1,0,1]*6],info=base+'0',tns=(False,rejected_order))
                    target='0010000'+'1'+base+'0'+'00'+2*channel(0,[0]*6,[[0]*24])
                    cce='0100001'+'0'+'000'+'1'+'0000'+'00'+'0'+'0'+'10'+body
                    raw=packed(cce+target+'111');row=dict(offset=len(blob),bytes=len(raw));blob.extend(raw)
                    refusal=dict(n=n,order=rejected_order,asc=asc,packet=row)
                    refusal['video']=video_fixture([dict(c,frames=[row],samples=n)],blob,channels=2,filename=f'{prefix}-{n}-refuse-{rejected_order}-synthetic.mp4')
                    refusals.append(refusal)

    for name,data in [('packets.bin',blob),('reference.f32le',gold),('incorrect-prediction.f32le',wrong)]: (DEST/f'{prefix}-{name}').write_bytes(data)
    (DEST/f'{prefix}.json').write_text(json.dumps(dict(cases=cases,refusals=refusals,provenance=('Multi-tap reflections +/-1, all order/resolution/compression combinations. ' if multitap else '')+'Own long-window orders 1..12, ' + ('multiple reflection coefficients' if multitap else 'last reflection coefficient 1') + ', six nonzero bands, PNS, LTP, signed gains, source/target TNS and distinct target residuals. Scalar direct cosine and absolute PCM histories; no FFmpeg, private media, foreign decoder or network.'),indent=2)+'\n')
    print('generated four long-order videos and controls')

if __name__=='__main__':main()
