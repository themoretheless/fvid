#!/usr/bin/env python3
"""Own AAC-LD rate/layout/high-band videos and sparse scalar PCM references."""
import json,math,struct
from generate_aac_main_tools_fixtures import sc,tuple_bits,f32
from generate_aac_ld_filterbank_fixtures import window
from generate_aac_ld_ltp_fixtures import GAINS
from generate_he_aac_packet_fixtures import DEST,field,frequency,packed,video_fixture

# Configured AAC element ordering; expected PCM is ascending WAVE speaker bits.
LAYOUTS={1:([0],[0],0x4),2:([1],[0,1],0x3),3:([0,1],[2,0,1],0x7),4:([0,1,0],[2,0,1,3],0x107),5:([0,1,1],[2,0,1,3,4],0x37),6:([0,1,1,3],[2,0,1,4,5,3],0x3f),7:([0,1,1,1,3],[2,6,7,0,1,4,5,3],0xff),11:([0,1,1,0,3],[2,0,1,4,5,6,3],0x13f),12:([0,1,1,1,3],[2,0,1,6,7,4,5,3],0x63f),14:([0,1,1,3,1],[2,0,1,4,5,3,6,7],0x503f)}
RATES=(22050,24000,32000,44100,48000,27713,37566,46009)
def geometry(n,rate):
    # ISO 4.86–4.91: full long-band count and last band's lower boundary.
    if rate>=37566:return (35,432) if n==480 else (36,460)
    if rate>=27713:return (37,448) if n==480 else (37,480)
    return (30,448) if n==480 else (31,480)

class Oracle:
    def __init__(self,n,high,cos):
        self.n=n;self.high=high;self.cos=cos;self.pcm=[];self.overlap=[0.]*n;self.shape=0;self.lag=0
    def run(self,q,gain,shape,active,update,coefficient):
        n=self.n;lag=self.lag if update is None else update
        values=[f32(v*2**((gain-100)/4)) for v in q]
        if active:
            estimate=[]
            for i in range(2*n):
                relative=i-n-lag;absolute=len(self.pcm)+relative
                value=self.overlap[relative] if relative>=0 else self.pcm[absolute] if absolute>=0 else 0.
                estimate.append(value*GAINS[coefficient]*window(n,i,self.shape if i<n else shape))
            for k in range(4):values[k]=f32(values[k]+sum(v*c for v,c in zip(estimate,self.cos[k])))
        transformed=[2/n*sum(values[k]*self.cos[k][i] for k in range(8)) for i in range(2*n)]
        out=[self.overlap[i]+transformed[i]*window(n,i,self.shape) for i in range(n)]
        self.overlap=[transformed[n+i]*window(n,n+i,shape) for i in range(n)]
        self.pcm.extend(out);self.shape=shape
        if active:self.lag=lag
        return [f32(v/65536) for v in out]

def body(n,count,high,frame,ch,lfe=False):
    if lfe:count=1
    gain=120+3*ch+frame%4;shape=(frame+ch)//2%2;active=frame>=2 and frame!=4
    update=(1023-17*ch if frame==2 else 13+11*ch) if active and frame in (2,6) else None
    coefficient=(frame+ch)%8;q=[(frame+ch+k)%3-1 for k in range(8)]
    if lfe:q[4:]=[0]*4
    info='000'+field(shape,1)+field(count,6)+field(active,1)
    if active:info+='1'+field(update is not None,1)+(field(update,10) if update is not None else '')+field(coefficient,3)+'1'+'0'*(count-1)
    books=[1] if lfe else [1]+[0]*(count-2)+[1]
    sections=''.join(field(book,4)+field(1,5) for book in books)
    spectral=tuple_bits(q[:4])+('' if lfe else tuple_bits(q[4:])+tuple_bits([0]*4)*((n-high)//4-1))
    return field(gain,8)+info+sections+sc(0)*sum(book!=0 for book in books)+'000'+spectral,(q,gain,shape,active,update,coefficient)

def main():
    blob=bytearray();gold=bytearray();cases=[]
    for n in (480,512):
        for rate in RATES:
            count,high=geometry(n,rate);bins=list(range(4))+list(range(high,high+4))
            cos=[[math.cos(math.pi/n*(i+.5+n/2)*(k+.5)) for i in range(2*n)] for k in bins]
            for config,(elements,mapping,mask) in LAYOUTS.items():
                width=len(mapping);banks=[Oracle(n,high,cos) for _ in range(width)];rows=[];start=len(gold)
                for frame in range(8):
                    wire='';pcm=[];ch=0
                    for element in elements:
                        wire+='0000'+('0' if element==1 else '')
                        for _ in range(2 if element==1 else 1):
                            encoded,parameters=body(n,count,high,frame,ch,lfe=element==3);wire+=encoded
                            pcm.append(banks[ch].run(*parameters));ch+=1
                    assert ch==width
                    raw=packed(wire);rows.append(dict(offset=len(blob),bytes=len(raw),reference_offset=len(gold)));blob.extend(raw)
                    for i in range(n):
                        row=[0.]*width
                        for ch,target in enumerate(mapping):row[target]=pcm[ch][i]
                        gold.extend(struct.pack('<'+'f'*width,*row))
                asc=packed(field(23,5)+frequency(rate)+field(config,4)+field(n==480,1)+'00'+'00').hex()
                name=f'{n}-{rate}-{config}';case=dict(name=name,n=n,rate=rate,configuration=config,channels=width,mask=mask,band_count=count,high_bin=high,asc=asc,frames=rows,reference_offset=start,reference_bytes=len(gold)-start,container_rate=rate,container_frame_samples=n,samples=8*n,pcm_offset=0,slots=n//64,bands=32)
                case['video']=video_fixture([case],blob,channels=width,filename=f'aac-ld-layout-{name}-synthetic.mp4');cases.append(case)
    (DEST/'aac-ld-layout-packets.bin').write_bytes(blob);(DEST/'aac-ld-layout-reference.f32le').write_bytes(gold)
    (DEST/'aac-ld-layout.json').write_text(json.dumps(dict(cases=cases,provenance='Own AOT23 ep0 indexed/explicit rates, all ten configured layouts, nonzero first and final scalefactor bands, independent LD histories/windows/lag updates. Sparse direct cosine and absolute timeline PCM oracle, reordered to WAVE speaker bits. No private media, FFmpeg, network or foreign decoder.'),indent=2)+'\n')
    print(f'generated {len(cases)} authored LD rate/layout/high-band videos')
if __name__=='__main__':main()
