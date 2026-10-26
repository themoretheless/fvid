"""Own nonzero encoded ELD/LD-SBR videos and independent direct PCM oracle.

Generation is explicit and offline. No foreign codec, subprocess or private media.
Initial constant-envelope matrix; not a claim of all LD-SBR tools coverage.
"""
import json,math,struct
from pathlib import Path
from generate_aac_eld_fixtures import generate as core_generate,mkv
from generate_aac_ld_layout_fixtures import LAYOUTS
from generate_he_aac_packet_fixtures import field,frequency,packed,video_fixture
from generate_aac_sbr_dsp_fixtures import header
from generate_aac_sbr_data_fixtures import word
from generate_aac_sbr_extension_fixtures import crc
from generate_aac_sbr_history_fixtures import row as entropy_row
from generate_aac_sbr_frequency_oracles import tables
from generate_aac_eld_cldfb_fixtures import window,synthesis
ROOT=Path(__file__).resolve().parents[1]
OUT=ROOT/'tests/fixtures/playback-errors/aac-eld-sbr'

def analysis(pcm,c,bands=10):
    ci=[(a+b)/2 for a,b in zip(c[::2],c[1::2])]
    weights=[[ci[lag]*2*complex(math.cos(math.pi*(k+.5)*(2*(lag%64)-96)/64),math.sin(math.pi*(k+.5)*(2*(lag%64)-96)/64))for lag in range(320)]for k in range(bands)]
    return [[sum(pcm[t*32+31-lag]*w[lag]for lag in range(min(320,t*32+32)))*32768 for w in weights]for t in range(len(pcm)//32)]

def grid(mode,slots,frame):
    coarse=mode=='coarse';fine=mode!='low'
    if mode.startswith('transient'):
        position=(frame+(8 if mode.endswith('8')else 0))%slots
        borders=[0,position+4]if position<2 else [0,position]+([position+4]if position<=9 or position==10 and slots==16 else [])
        borders.append(slots);count=len(borders)-1
        return '1'+field(position,4)+field(fine,1)*count,borders,0 if position<2 else 1,False,fine
    count={'two':2,'four':4}.get(mode,1)
    # FIXFIX borders use nearest integer placement, including 15-slot frames.
    borders=[i*((slots+count//2)//count)for i in range(count)]+[slots]
    return '0'+field({1:0,2:1,4:2}[count],2)+(field(coarse,1)if count==1 else '')+field(fine,1),borders,None,coarse,fine

def reconstruct(low,slots,noise,mode='base',channel=0):
    source=list(range(2,10))+list(range(2,9))+[None,None]
    target=128.;ratio=.5
    if mode=='coupled-pan':
        target=256/(1+2**(2 if channel==0 else -2))
        ratio=1/(1+2**(-4 if channel==0 else 4))
    output=[];index=0;bandwidth=0.;previous_mode=0;gain_history=[];noise_history=[]
    _,hb,_,_=tables(10,27,0,False,0,0)
    centers={(a+b)//2 for a,b in zip(hb,hb[1:])}
    for frame,start in enumerate(range(0,len(low),slots)):
        rows=low[start:start+slots]
        hf=[[r[k]if k is not None else 0j for k in source]for r in rows]
        if mode=='invf':
            current_mode=frame%4
            target_bw=[.6 if previous_mode==1 else 0.,.6 if previous_mode==0 else .75,.9,.98][current_mode]
            bandwidth=(.75*target_bw+.25*bandwidth)if target_bw<bandwidth else .90625*target_bw+.09375*bandwidth
            if bandwidth<.015625:bandwidth=0.
            previous_mode=current_mode
            for b,k in enumerate(source):
                if k is None:continue
                x=[low[t][k]if t>=0 else 0j for t in range(start-2,start+slots)]
                phi=lambda i,j:sum(x[t-i]*x[t-j].conjugate()for t in range(2,len(x)))
                p01,p02,p12=phi(0,1),phi(0,2),phi(1,2)
                p11,p22=phi(1,1).real,phi(2,2).real
                det=p11*p22-abs(p12)**2/1.000001
                a2=(p01*p12-p02*p11)/det if det else 0j
                a1=-(p01+a2*p12.conjugate())/p11 if p11 else 0j
                if abs(a1)**2>=16 or abs(a2)**2>=16:a1=a2=0j
                for t in range(slots):hf[t][b]=x[t+2]+bandwidth*a1*x[t+1]+bandwidth**2*a2*x[t]

        _,borders,attack,_,_=grid(mode,slots,frame)
        for env,(first,last) in enumerate(zip(borders,borders[1:])):
            suppress=env==attack
            energy=[sum(abs(r[b])**2 for r in hf[first:last])/(last-first)for b in range(17)]
            harmonic=mode=='harmonic'and frame%3!=2
            gain=[math.sqrt(target*(ratio/(1+ratio)if harmonic else 1 if suppress else 1/(1+ratio))/(1+e))for e in energy]
            sine=[math.sqrt(target/(1+ratio))if harmonic and b+10 in centers else 0. for b in range(17)]
            q=[math.sqrt(target*ratio/(1+ratio))]*17
            maximum=min(math.sqrt((17*target+1e-12)/(sum(energy)+1e-12))*1.41254,1e5)
            for b,g in enumerate(gain):
                if g>maximum:q[b]*=maximum/g;gain[b]=maximum
            boost=min(math.sqrt((17*target+1e-12)/(sum(e*g*g+v*v+(0 if suppress or v else n*n)for e,g,n,v in zip(energy,gain,q,sine))+1e-12)),1.584893192)
            gain=[v*boost for v in gain];q=[v*boost for v in q];sine=[v*boost for v in sine]
            if not gain_history:gain_history=[gain[:]]*4;noise_history=[q[:]]*4
            for time,(row,h) in enumerate(zip(rows[first:last],hf[first:last]),start+first):
                adjusted=[]
                for b in range(17):
                    index=(index+1)%512
                    smooth=[.33333333333333,.30150283239582,.21816949906249,.11516383427084,.03183050093751]
                    g=sum(w*v for w,v in zip(smooth,[gain[b]]+[r[b]for r in gain_history]))if mode=='smoothing'else gain[b]
                    qn=sum(w*v for w,v in zip(smooth,[q[b]]+[r[b]for r in noise_history]))if mode=='smoothing'else q[b]
                    phase=[1,1j,-1,-1j][time%4]
                    if (b+10)%2:phase=phase.conjugate()
                    adjusted.append(h[b]*g+(0 if suppress or sine[b]else noise[index]*qn)+sine[b]*phase)
                output.append(row+adjusted+[0j]*37)
                gain_history=[gain[:]]+gain_history[:3];noise_history=[q[:]]+noise_history[:3]
    return output

def sbr_bits(channels,nhigh,nlow,slots,frame,protected,mode='base'):
    coupled=channels==2 and mode.startswith('coupled')
    syntax,borders,_,coarse,fine=grid(mode,slots,frame)
    count=len(borders)-1;nq=1 if count==1 else 2
    temporal=mode=='base'and frame>0
    flags=field(temporal,1)*(count+nq)
    envelopes=[];noises=[]
    for ch in range(channels):
        balance=coupled and ch==1
        e=(10 if mode=='coupled-pan'else 12)if balance else (1 if coarse else 2)
        q=(4 if mode=='coupled-pan'else 6)if balance else 7
        bands=nhigh if fine else nlow
        envelopes.append(''.join(entropy_row(([0]*bands if temporal else [e]+[0]*(bands-1)),temporal,coarse,balance,False)for _ in range(count)))
        noises.append(''.join(entropy_row([0 if temporal else q],temporal,False,balance,True)for _ in range(nq)))
    data='0'+(field(coupled,1)if channels==2 else '')+syntax*(1 if coupled else channels)+flags*channels+field(frame%4 if mode=='invf'else 0,2)*(1 if coupled else channels)
    data+=(''.join(a+b for a,b in zip(envelopes,noises))if coupled else ''.join(envelopes)+''.join(noises))+('1'+'1'*nhigh if mode=='harmonic'and frame%3!=2 else '0')*channels+'0'
    present=mode=='header'and frame in [0,4,8]
    body=field(present,1)+(header(0,True)if present else '')+data
    return (field(crc(body),10)if protected else '')+body

def generate():
    core=core_generate(True,[(n,24000,c,'base',0,0,False)for n in [512,480]for c in LAYOUTS]);manifest=json.loads(core['manifest.json']);c=window()
    noise=[complex(*struct.unpack_from('<dd',(ROOT/'tests/fixtures/playback-errors/aac-sbr-noise-protocol.f64le').read_bytes(),i*16))for i in range(512)]
    _,high,low,_=tables(10,27,0,False,0,0);nhigh=len(high)-1;nlow=len(low)-1
    files={};cases=[];blob=bytearray()
    for base in manifest['cases']:
        if base['tool']!='base':continue
        n=base['n'];channels=base['channels'];configuration=base['configuration']
        original=list(struct.unpack('<'+'f'*(len(core[base['name']+'.f32le'])//4),core[base['name']+'.f32le']))
        elements,mapping,_=LAYOUTS[configuration]
        lfe=mapping[sum(2 if e==1 else 1 for e in elements[:elements.index(3)])]if 3 in elements else None
        rows=[([r+[0j]*32 for r in analysis(original[ch::channels],c,32)]if ch==lfe else reconstruct(analysis(original[ch::channels],c),n//32,noise))for ch in range(channels)]
        for double in [False,True]:
            bands=64 if double else 32;rate=24000*(1+double)
            planar=[[v/(32768 if double else 65536)for v in synthesis(r,bands,c)]for r in rows]
            pcm=b''.join(struct.pack('<'+'f'*channels,*(p[i]for p in planar))for i in range(len(planar[0])))
            for protected in [False,True]:
                name=f'n{n}-c{configuration}-x{1+double}-crc{int(protected)}'
                config=packed(field(31,5)+field(7,6)+frequency(24000)+field(configuration,4)+field(n==480,1)+'000'+'1'+field(double,1)+field(protected,1)+header(0,True)*sum(e!=3 for e in elements)+'0000'+'00')
                frames=[];packets=[]
                for i,row in enumerate(base['frames']):
                    bits=row['bits']+''.join(sbr_bits(2 if e==1 else 1,nhigh,nlow,n//32,i,protected)for e in elements if e!=3)
                    packet=packed(bits);frames.append(dict(offset=len(blob),bytes=len(packet)));blob.extend(packet);packets.append(packet)
                case=dict(name=name,n=n,rate=rate,configuration=configuration,mask=base['mask'],asc=config.hex(),channels=channels,frames=frames,container_rate=rate,container_frame_samples=n*(1+double),samples=len(planar[0]),pcm_offset=0,slots=n//32,bands=bands)
                files[name+'.asc']=config;files[name+'.f32le']=pcm
                files[name+'.mp4']=video_fixture([case],blob,channels,filename=name+'.mp4',write=False)['data'];case['video']=name+'.mp4'
                files[name+'.mka']=mkv(config,packets,rate,n*(1+double),channels);case['matroska']=name+'.mka';cases.append(case)
    for n in [480,512]:
        base=next(b for b in manifest['cases']if b['n']==n and b['configuration']==2)
        original=list(struct.unpack('<'+'f'*(len(core[base['name']+'.f32le'])//4),core[base['name']+'.f32le']))
        lows=[analysis(original[ch::2],c)for ch in range(2)]
        for mode in ['two','four','coarse','low','transient0','transient8','coupled','coupled-pan','invf','smoothing','harmonic','header']:
            name=f'n{n}-c2-{mode}'
            config=packed(field(31,5)+field(7,6)+frequency(24000)+field(2,4)+field(n==480,1)+'000'+'111'+header(0,mode!='smoothing')+'0000'+'00')
            planar=[[v/32768 for v in synthesis(reconstruct(low,n//32,noise,mode,ch),64,c)]for ch,low in enumerate(lows)]
            frames=[];packets=[]
            for i,row in enumerate(base['frames']):
                packet=packed(row['bits']+sbr_bits(2,nhigh,nlow,n//32,i,True,mode));frames.append(dict(offset=len(blob),bytes=len(packet)));blob.extend(packet);packets.append(packet)
            case=dict(name=name,n=n,rate=48000,configuration=2,mask=3,asc=config.hex(),channels=2,frames=frames,container_rate=48000,container_frame_samples=n*2,samples=len(planar[0]),pcm_offset=0,slots=n//32,bands=64)
            files[name+'.asc']=config;files[name+'.f32le']=b''.join(struct.pack('<ff',*r)for r in zip(*planar))
            files[name+'.mp4']=video_fixture([case],blob,2,filename=name+'.mp4',write=False)['data'];case['video']=name+'.mp4'
            files[name+'.mka']=mkv(config,packets,48000,n*2,2);case['matroska']=name+'.mka';cases.append(case)
    malformed=[]
    for n in [480,512]:
        base=next(b for b in manifest['cases']if b['n']==n and b['configuration']==1)
        good=next(b for b in cases if b['name']==f'n{n}-c1-x2-crc1')
        for kind in ['crc','truncated']:
            name=f'n{n}-{kind}'
            packets=[];frames=[]
            for i,row in enumerate(base['frames']):
                bits=sbr_bits(1,nhigh,nlow,n//32,i,True)
                if i==3 and kind=='crc':bits=('1'if bits[0]=='0'else '0')+bits[1:]
                packet=packed(row['bits']+bits)
                if i==3 and kind=='truncated':packet=packet[:-1]
                frames.append(dict(offset=len(blob),bytes=len(packet)));blob.extend(packet);packets.append(packet)
            case=dict(good,name=name,frames=frames)
            filename=name+'.mp4';files[filename]=video_fixture([case],blob,1,filename=filename,write=False)['data']
            malformed.append(dict(name=name,video=filename,asc=good['asc'],frames=frames,error='CRC'if kind=='crc'else 'truncated'))
    files['packets.bin']=bytes(blob)
    files['manifest.json']=(json.dumps(dict(cases=cases,malformed=malformed,provenance='Own encoded nonzero ELD core, LD-SBR syntax/CRC and direct CLDFB convolution, patch/energy/gain/noise and column-superposition oracle; no production decoder, foreign codec, process, private media or network.'),indent=2)+'\n').encode()
    return files
if __name__=='__main__':
    OUT.mkdir(exist_ok=True)
    for name,data in generate().items():(OUT/name).write_bytes(data)
