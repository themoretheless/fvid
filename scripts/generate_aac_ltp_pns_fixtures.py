#!/usr/bin/env python3
"""Authored AOT4/19/23 PNS precedence reproducers, controls and scalar PCM."""
import json,math,struct
from generate_aac_main_tools_fixtures import channel,Noise,f32
from generate_aac_ld_filterbank_fixtures import window
from generate_aac_ld_ltp_fixtures import GAINS
from generate_he_aac_packet_fixtures import DEST,field,frequency,packed,video_fixture

class Oracle:
    def __init__(self,n,ld,cos):
        self.n=n;self.ld=ld;self.cos=cos;self.pcm=[];self.overlap=[0.]*n;self.shape=0
    def weight(self,i,shape):
        return window(self.n,i,shape) if self.ld else math.sin(math.pi*(i+.5)/(2*self.n))
    def run(self,residual,shape,active,lag,coefficient,used):
        n=self.n;values=residual[:]
        if active:
            estimate=[]
            for i in range(2*n):
                relative=i-(n if self.ld else 0)-lag;absolute=len(self.pcm)+relative
                value=self.overlap[relative] if 0<=relative<n else self.pcm[absolute] if relative<0 and absolute>=0 else 0.
                estimate.append(value*GAINS[coefficient]*self.weight(i,self.shape if i<n else shape))
            for k in range(8):
                if used[k//4]:values[k]=f32(values[k]+sum(v*c for v,c in zip(estimate,self.cos[k])))
        transformed=[2/n*sum(values[k]*self.cos[k][i] for k in range(8)) for i in range(2*n)]
        out=[self.overlap[i]+transformed[i]*self.weight(i,self.shape) for i in range(n)]
        self.overlap=[transformed[n+i]*self.weight(n+i,shape) for i in range(n)]
        self.pcm.extend(out);self.shape=shape
        return [f32(v/65536) for v in out]

def prediction(aot,active,lag,coef,used):
    return field(active,1)+(('1'+field(lag,10) if aot==23 else field(lag,11))+field(coef,3)+''.join(field(v,1) for v in used) if active else '')

def main():
    blob=bytearray();gold=bytearray();wrong=bytearray();cases=[]
    for aot in (4,19,23):
        for n in ((480,512) if aot==23 else (960,1024)):
            cos=[[math.cos(math.pi/n*(i+.5+n/2)*(k+.5)) for i in range(2*n)] for k in range(8)]
            for mode in (('mono','independent-right','correlated','uncorrelated','coupling') if aot==4 else ('mono','independent-right','correlated','uncorrelated')):
                width=1 if mode in ('mono','coupling') else 2;common=mode in ('correlated','uncorrelated');ms=1 if mode=='correlated' else 0
                banks=[Oracle(n,aot==23,cos) for _ in range(width)];mutants=[Oracle(n,aot==23,cos) for _ in range(width)]
                noise=Noise();rows=[];control_rows=[];start=len(gold)
                for frame in range(12):
                    shape=frame%2 if aot==23 else 0;active=frame>=1;books=[[1,1] for _ in range(width)]
                    if 3<=frame<=8:
                        books[-1][1]=13
                        if common:books[0][1]=13
                    q=[[(frame+k+ch)%3-1 for k in range(8)] for ch in range(width)]
                    energy=[50+4*ch for ch in range(width)];lags=[1023-17*ch if aot==23 else n-13*(ch+1) for ch in range(width)];coefs=[(frame+3*ch)%8 for ch in range(width)]
                    pred=[prediction(aot,active,lags[ch],coefs[ch],[True,True]) for ch in range(width)]
                    control_pred=[prediction(aot,active,lags[ch],coefs[ch],[True,books[ch][1]!=13]) for ch in range(width)]
                    base='000'+field(shape,1)+field(2,6)
                    def encode(prediction):
                        wire=('0010000' if width==2 else '0000000') if aot==4 else '0000'
                        if width==2:wire+=field(common,1)
                        if common:
                            wire+=base+field(active,1)
                            if aot==4 and active:wire+=''.join(prediction)
                            wire+=field(ms,2)+('01' if ms==1 else '')
                            if aot!=4 and active:wire+=prediction[0]
                            wire+=channel(0,books[0],[q[0]],energy=energy[0])
                            if aot!=4 and active:wire+=prediction[1]
                            wire+=channel(0,books[1],[q[1]],energy=energy[1])
                        else:
                            for ch in range(width):
                                info=base+field(active,1)+(prediction[ch] if active else '')
                                wire+=channel(0,books[ch],[q[ch]],energy=energy[ch],info=info)
                        if mode=='coupling':
                            # Independent CCE tag1 -> silent SCE tag0, unity gain.
                            source=wire[7:]
                            target='0000000'+channel(0,[0,0],[[0]*8],info=base+'0')
                            cce='0100001'+'1'+'000'+'0'+'0000'+'0'+'0'+'10'+source
                            wire=cce+target if frame%2 else target+cce
                        return packed(wire+('111' if aot==4 else ''))
                    raw=encode(pred);rows.append(dict(offset=len(blob),bytes=len(raw),reference_offset=len(gold)));blob.extend(raw)
                    control=encode(control_pred);control_rows.append(dict(offset=len(blob),bytes=len(control),reference_offset=len(gold)));blob.extend(control)
                    spectra=[[float(v*1024) for v in row] for row in q]
                    for ch in range(width):
                        if books[ch][1]==13:spectra[ch][4:]=noise.band(energy[ch])
                    if mode=='correlated':
                        for k in range(4,8):
                            if books[0][1]==books[1][1]==13:spectra[1][k]=f32(spectra[0][k]*2**((energy[1]-energy[0])/4))
                            else:
                                a,b=spectra[0][k],spectra[1][k];spectra[0][k]=f32(a+b);spectra[1][k]=f32(a-b)
                    pcm=[banks[ch].run(spectra[ch],shape,active,lags[ch],coefs[ch],[True,books[ch][1]!=13]) for ch in range(width)]
                    mutant=[mutants[ch].run(spectra[ch],shape,active,lags[ch],coefs[ch],[True,True]) for ch in range(width)]
                    for values in zip(*pcm):gold.extend(struct.pack('<'+'f'*width,*values))
                    for values in zip(*mutant):wrong.extend(struct.pack('<'+'f'*width,*values))
                asc=packed(field(aot,5)+frequency(24000)+field(width,4)+field(n in (480,960),1)+'00'+('00' if aot!=4 else '')).hex()
                if mode=='coupling':
                    prefix=field(4,5)+frequency(24000)+'0000'+field(n==960,1)+'00'
                    pce=field(0,4)+field(3,2)+frequency(24000)+field(1,4)+field(0,4)+field(0,4)+field(0,2)+field(0,3)+field(1,4)+'000'+'0'+field(0,4)+'1'+field(1,4)
                    asc=packed(prefix+pce+'0'*(-len(prefix+pce)%8)+field(0,8)).hex()
                name=f'{aot}-{n}-{mode}';c=dict(name=name,aot=aot,n=n,channels=width,asc=asc,frames=rows,control_frames=control_rows,reference_offset=start,reference_bytes=len(gold)-start,container_rate=24000,container_frame_samples=n,samples=12*n,pcm_offset=0,slots=n//64,bands=32)
                c['video']=video_fixture([c],blob,channels=width,filename=f'aac-ltp-pns-{name}-synthetic.mp4')
                c['control_video']=video_fixture([dict(c,frames=control_rows)],blob,channels=width,filename=f'aac-ltp-pns-{name}-control-synthetic.mp4');cases.append(c)
    (DEST/'aac-ltp-pns-packets.bin').write_bytes(blob);(DEST/'aac-ltp-pns-reference.f32le').write_bytes(gold);(DEST/'aac-ltp-pns-incorrect-prediction.f32le').write_bytes(wrong)
    (DEST/'aac-ltp-pns.json').write_text(json.dumps(dict(cases=cases,provenance='Own AOT4/19/23 long-window PNS precedence packets with active overlapping LTP flags and matched controls clearing only those flags. Original scalar PNS, direct cosine/timeline/LD or sine overlap PCM plus deliberately incorrect PNS-prediction mutant. No private media, FFmpeg, network or foreign decoder.'),indent=2)+'\n')
    print(f'generated {len(cases)} PNS precedence videos and {len(cases)} matched controls')
if __name__=='__main__':main()
