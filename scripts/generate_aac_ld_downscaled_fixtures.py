"""Own AAC-LD low-rate movies with nominal-clock LTP and direct short synthesis.
Explicit offline generation; no decoder, process, network or private media.
"""
import json,math,struct
from functools import lru_cache
from pathlib import Path
from generate_aac_ld_layout_fixtures import LAYOUTS,RATES,geometry,body
from generate_aac_ld_filterbank_fixtures import window
from generate_aac_ld_ltp_fixtures import GAINS
from generate_aac_main_tools_fixtures import f32
from generate_he_aac_packet_fixtures import field,frequency,packed,video_fixture
from generate_aac_eld_fixtures import mkv
from generate_mp4_ep1_grouped_ps_fixtures import replace_asc
ROOT=Path(__file__).resolve().parents[1];OUT=ROOT/'tests/fixtures/playback-errors/aac-ld-downscaled'
@lru_cache(None)
def basis(n,bins):return [[math.cos(math.pi/n*(i+.5+n/2)*(k+.5))for i in range(2*n)]for k in bins]
class Source:
    def __init__(self,n,last):
        self.n=n;self.positions=tuple(range(4))+tuple(range(last,last+4));self.cos=basis(n,self.positions);self.pcm=[];self.overlap=[0.]*n;self.shape=0;self.lag=0
    def run(self,q,gain,shape,active,update,coefficient):
        n=self.n;lag=self.lag if update is None else update;values=[f32(v*2**((gain-100)/4))for v in q]
        if active:
            # Absolute nominal PCM timeline, not a shifted production buffer.
            estimate=[]
            for i in range(2*n):
                relative=i-n-lag;at=len(self.pcm)+relative
                v=self.overlap[relative]if relative>=0 else self.pcm[at]if at>=0 else 0.
                estimate.append(v*GAINS[coefficient]*window(n,i,self.shape if i<n else shape))
            for k in range(4):values[k]=f32(values[k]+sum(v*c for v,c in zip(estimate,self.cos[k])))
        transform=[2/n*sum(v*self.cos[k][i]for k,v in enumerate(values))for i in range(2*n)]
        out=[self.overlap[i]+transform[i]*window(n,i,self.shape)for i in range(n)]
        self.overlap=[transform[n+i]*window(n,n+i,shape)for i in range(n)];self.pcm.extend(out);self.shape=shape
        if active:self.lag=lag
        return values,[f32(v/65536)for v in out]
def short_pcm(n,sn,positions,frames):
    overlap=[0.]*sn;before=0;pcm=[];low=[(i,p)for i,p in enumerate(positions)if p<sn];cos=basis(sn,tuple(p for _,p in low))
    for values,shape in frames:
        # Source (nominal) normalization, with a genuinely short inverse cosine.
        transform=[2/n*sum(values[index]*cos[k][i]for k,(index,_)in enumerate(low))for i in range(2*sn)]
        pcm.extend(f32((overlap[i]+transform[i]*window(sn,i,before))/65536)for i in range(sn))
        overlap=[transform[sn+i]*window(sn,sn+i,shape)for i in range(sn)];before=shape
    return pcm
def interleave(planar,mapping):return b''.join(struct.pack('<'+'f'*len(mapping),*(planar[mapping.index(ch)][i]for ch in range(len(mapping))))for i in range(len(planar[0])))
def generate():
    files={};cases=[];exports=[];blob=bytearray()
    for n in [480,512]:
     for rate in RATES:
      count,last=geometry(n,rate)
      for cfg,(elements,mapping,mask)in LAYOUTS.items():
        valid=[f for f in [2,3,4,8]if n%f==0 and rate%f==0]
        if not valid:continue
        banks=[Source(n,last)for _ in mapping];wire_rows=[];spectra=[[]for _ in mapping];full=[[]for _ in mapping]
        for frame in range(8):
            wire='';ch=0;tags={}
            for element in elements:
                tag=tags.get(element,0);tags[element]=tag+1;wire+=field(tag,4)+('0'if element==1 else '')
                for _ in range(2 if element==1 else 1):
                    encoded,parameters=body(n,count,last,frame,ch,lfe=element==3);wire+=encoded;values,pcm=banks[ch].run(*parameters);spectra[ch].append((values,parameters[2]));full[ch].extend(pcm);ch+=1
            wire_rows.append(wire)
        asc_bits=field(23,5)+frequency(rate)+field(cfg,4)+field(n==480,1)+'00'+'00';asc=packed(asc_bits)
        full_name=f'n{n}-r{rate}-c{cfg}.nominal.f32le';files[full_name]=interleave(full,mapping)
        for factor in valid:
            sn=n//factor;name=f'n{n}-r{rate}-c{cfg}-ds{factor}';frames=[];packets=[]
            for wire in wire_rows:
                packet=packed(wire);frames.append(dict(offset=len(blob),bytes=len(packet)));blob.extend(packet);packets.append(packet)
            planar=[short_pcm(n,sn,b.positions,rows)for b,rows in zip(banks,spectra)]
            case=dict(name=name,n=n,asc=asc.hex(),frames=frames,channels=len(mapping),mask=mask,configuration=cfg,factor=factor,source_rate=rate,rate=rate//factor,out_n=sn,container_rate=rate//factor,container_frame_samples=sn,slots=n//64,bands=32,samples=8*sn,pcm_offset=0,video=name+'.mp4',matroska=name+'.mka',golden=name+'.f32le',nominal=full_name)
            files[case['golden']]=interleave(planar,mapping);files[case['video']]=video_fixture([case],blob,len(mapping),filename=case['video'],write=False)['data'];files[case['matroska']]=mkv(asc,packets,rate//factor,sn,len(mapping));cases.append(case)
            ep=packed(asc_bits[:-2]+'01');export=dict(case,name='ep1-'+name,asc=ep.hex(),video='ep1-'+name+'.mp4',control=case['video']);files[export['video']]=replace_asc(files[case['video']],ep);exports.append(export)
    files['packets.bin']=bytes(blob);files['manifest.json']=(json.dumps(dict(cases=cases,exports=exports,provenance='Own tagged AAC-LD first/final SFBs, alternating windows, lag 1023, odd/retained lag and all LTP gains. Absolute nominal prediction timeline, direct full/short cosine and analytical windows. No private media, foreign decoder, process or network.'),indent=2)+'\n').encode();return files
if __name__=='__main__':
    OUT.mkdir(exist_ok=True)
    for name,data in generate().items():(OUT/name).write_bytes(data)
