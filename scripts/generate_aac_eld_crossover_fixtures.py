"""Own dynamic LF-bank crossover movies and direct complex PCM oracle.
Generation is explicit and offline; no decoder, process, network or private media.
"""
import copy,json,math,struct
from functools import lru_cache
from generate_aac_eld_fixtures import generate as core,mkv
from generate_aac_eld_short_fixtures import short_window
from generate_aac_eld_downscale_info_fixtures import extension
from generate_aac_eld_cldfb_downscaled_fixtures import ROOT,analysis,synthesis
from generate_aac_eld_sbr_fixtures import sbr_bits
from generate_aac_sbr_dsp_fixtures import header
from generate_aac_sbr_frequency_oracles import tables
from generate_aac_sbr_extension_fixtures import crc
from generate_he_aac_packet_fixtures import packed,field,frequency,video_fixture
from generate_mp4_ep1_grouped_ps_fixtures import replace_asc
SOURCE=ROOT/'tests/fixtures/playback-errors';OUT=SOURCE/'aac-eld-crossover'
# Explicit original patch geometries, including the discarded final two bins.
GEOMETRY={
 8:[(4,8,3,0,[2,3,None,None]),(5,10,4,0,[1,2,3,None,None])],
 4:[(7,14,0,1,list(range(2,6))+[None,None]),(7,14,0,2,list(range(3,6))+[None,None])],
 2:[(15,30,7,1,list(range(2,14))+[None,None]),(15,30,7,2,list(range(3,14))+[None,None])],
}
def source_rate(factor):return 96000 if factor==8 else 24000
@lru_cache(None)
def geometry(factor,which):
    k0,stop,start,cross,sources=GEOMETRY[factor][which];_,high,low,_=tables(k0,stop,0,False,cross,0)
    assert len(sources)==stop-high[0]
    return dict(kx=high[0],stop=stop,start=start,cross=cross,sources=sources,high=high,low=low)
@lru_cache(None)
def signal(n,cfg,downscale,rate):
    data=core(True,[(n,rate,cfg,'base',0,0,False)],output_samples=n//downscale,synthesis_window=short_window if downscale!=1 else None,include_category_bits=True)
    case=json.loads(data['manifest.json'])['cases'][0];pcm=struct.unpack('<'+'f'*(len(data[case['name']+'.f32le'])//4),data[case['name']+'.f32le'])
    return case,pcm
@lru_cache(None)
def lows(n,cfg,ch,full,factor):
    case,pcm=signal(n,cfg,1 if full else factor,source_rate(factor))
    return [[v*32768 for v in row]for row in analysis(pcm[ch::cfg]if full else [v*factor for v in pcm[ch::cfg]],32 if full else 32//factor)]
def head(which,factor):
    g=geometry(factor,which);h=header(0,True)
    return h[:1]+field(g['start'],4)+field(14,4)+field(g['cross'],3)+h[12:]
def predictor(x):
    phi=lambda i,j:sum(x[t-i]*x[t-j].conjugate()for t in range(2,len(x)))
    a,b,c=phi(0,1),phi(0,2),phi(1,2);y,z=phi(1,1).real,phi(2,2).real
    det=y*z-abs(c)**2/1.000001;second=(a*c-b*y)/det if det else 0j;first=-(a+second*c.conjugate())/y if y else 0j
    return (0j,0j)if abs(first)**2>=16 or abs(second)**2>=16 else (first,second)
def oracle(n,cfg,ch,initial,mode,noise,factor):
    slots=n//32;past=[[0j]*32]*2;output=[];bandwidth=0.;noise_index=0
    for frame in range(12):
        which=initial^((frame//4)%2);g=geometry(factor,which);kx,stop,source=g['kx'],g['stop'],g['sources'];width=stop-kx
        # Analysis states advance over the entire core; select actual emitted LF.
        rows=[r[:kx]+[0j]*(32-kx)for r in lows(n,cfg,ch,kx>32//factor,factor)[frame*slots:(frame+1)*slots]]
        if frame%4==0:bandwidth=0.;noise_index=0
        bandwidth=.90625*(.98 if mode else 0.)+.09375*bandwidth
        if bandwidth<.015625:bandwidth=0.
        window=past+rows;hf=[[0j]*width for _ in rows]
        for band,k in enumerate(source):
            if k is None:continue
            x=[r[k]for r in window];a,b=predictor(x)if bandwidth else (0j,0j)
            for t in range(slots):hf[t][band]=x[t+2]+bandwidth*a*x[t+1]+bandwidth**2*b*x[t]
        energy=[sum(abs(r[b])**2 for r in hf)/slots for b in range(width)]
        gain=[math.sqrt(128/(1.5*(1+e)))for e in energy];q=[math.sqrt(128*.5/1.5)]*width
        maximum=min(math.sqrt((width*128+1e-12)/(sum(energy)+1e-12))*1.41254,1e5)
        for b,g in enumerate(gain):
            if g>maximum:q[b]*=maximum/g;gain[b]=maximum
        boost=min(math.sqrt((width*128+1e-12)/(sum(e*g*g+v*v for e,g,v in zip(energy,gain,q))+1e-12)),1.584893192)
        for row,h in zip(rows,hf):
            adjusted=[]
            for b in range(width):noise_index=(noise_index+1)%512;adjusted.append(boost*(gain[b]*h[b]+q[b]*noise[noise_index]))
            output.append(row[:kx]+adjusted+[0j]*(64-stop))
        past=rows[-2:]
    return output

def generate(factors=(8,)):
    files={};cases=[];exports=[];blob=bytearray();protected_sources=[]
    noise=[complex(*struct.unpack_from('<dd',(SOURCE/'aac-sbr-noise-protocol.f64le').read_bytes(),i*16))for i in range(512)]
    for factor in factors:
     for n in [480,512]:
      for cfg in [1,2]:
       base,_=signal(n,cfg,1,source_rate(factor))
       for initial in [0,1]:
        for mode in [0,3]:
         rows=[oracle(n,cfg,ch,initial,mode,noise,factor)for ch in range(cfg)]
         for double in [False,True]:
             name=f'n{n}-c{cfg}-start{initial}-invf{mode}-x{1+double}'+(''if factor==8 else f'-ds{factor}')
             bits=field(31,5)+field(7,6)+frequency(source_rate(factor))+field(cfg,4)+field(n==480,1)+'000'+'1'+field(double,1)+'1'+head(initial,factor)+extension(source_rate(factor)//factor,True)+'0000'+'00'
             asc=packed(bits);frames=[];packets=[];side_rows=[]
             for i,row in enumerate(base['frames']):
                 which=initial^((i//4)%2);g=geometry(factor,which);high,low=g['high'],g['low']
                 payload=sbr_bits(cfg,len(high)-1,len(low)-1,n//32,mode,False,'invf')
                 if i in [4,8]:payload='1'+head(which,factor)+payload[1:]
                 segments=sbr_bits(cfg,len(high)-1,len(low)-1,n//32,mode,False,'invf',True)
                 controls=segments[0]['bits']
                 if i in [4,8]:controls='1'+head(which,factor)+controls[1:]
                 assert controls+segments[1]['bits']==payload
                 segments[0]['bits']=field(crc(payload),10)+controls
                 side=copy.deepcopy(row['esc'])+segments
                 payload=field(crc(payload),10)+payload
                 packet_bits=row['bits']+payload;assert ''.join(p['bits']for p in side)==packet_bits;side[-1]['bits']+='0'*(-len(packet_bits)%8);side_rows.append(dict(esc=side));packet=packed(packet_bits);packets.append(packet);frames.append(dict(offset=len(blob),bytes=len(packet),end_bits=len(packet_bits)));blob.extend(packet)
             rate=source_rate(factor)//factor*(1+double);out_n=n//factor*(1+double)
             planar=[[v/32768 for v in synthesis(r,32//factor*(1+double))]for r in rows]
             case=dict(name=name,n=n,slots=n//32,bands=64 if double else 32,asc=asc.hex(),frames=frames,rate=rate,container_rate=rate,container_frame_samples=out_n,channels=cfg,mask=4 if cfg==1 else 3,samples=len(planar[0]),pcm_offset=0,out_n=out_n,source_rate=source_rate(factor)*(1+double),factor=factor,video=name+'.mp4',matroska=name+'.mka',golden=name+'.f32le',edges=[4*out_n,8*out_n],initial=initial,inverse_filter=mode)
             files[case['golden']]=b''.join(struct.pack('<'+'f'*cfg,*(p[i]for p in planar))for i in range(len(planar[0])))
             files[case['video']]=video_fixture([case],blob,cfg,filename=case['video'],write=False)['data'];files[case['matroska']]=mkv(asc,packets,rate,out_n,cfg);cases.append(case)
             asc_ep=packed(bits[:-2]+'01');ep=dict(case,name='ep1-'+name,asc=asc_ep.hex(),video='ep1-'+name+'.mp4',control=case['video'])
             files[ep['video']]=replace_asc(files[case['video']],asc_ep);exports.append(ep);protected_sources.append((case,bits,side_rows))
    from generate_aac_eld_esc_fixtures import configuration,protect
    for base,bits,side_rows in protected_sources:
        patterns=list(dict.fromkeys(tuple(p['category']for p in row['esc'])for row in side_rows));orders=[list(range(len(p)))[::-1]for p in patterns]
        _,_,_,ep=configuration(dict(base,rate=source_rate(base['factor']),configuration=base['channels'],flags=0),patterns,orders)
        asc=packed(bits[:-2]+'11'+ep+'1');frames=[];packets=[]
        for row in side_rows:
            packet,_=protect(row,patterns,orders);frames.append(dict(offset=len(blob),bytes=len(packet)));blob.extend(packet);packets.append(packet)
        name='ep3-'+base['name'];case=dict(base,name=name,asc=asc.hex(),frames=frames,video=name+'.mp4',matroska=name+'.mka',protected=True)
        files[case['video']]=video_fixture([case],blob,case['channels'],filename=case['video'],write=False)['data'];files[case['matroska']]=mkv(asc,packets,case['rate'],case['out_n'],case['channels']);cases.append(case)
    files['packets.bin']=bytes(blob);files['manifest.json']=(json.dumps(dict(cases=cases,exports=exports,provenance='Own nonzero 96-kHz ELD spectra, encoded alternating headers, direct convolution and modulated-column oracle. Predictor predecessor rows are the LF rows actually emitted by preceding frames. No private media, foreign decoder, process or network.'),indent=2)+'\n').encode();return files
if __name__=='__main__':
    OUT.mkdir(exist_ok=True)
    for name,data in generate().items():(OUT/name).write_bytes(data)
