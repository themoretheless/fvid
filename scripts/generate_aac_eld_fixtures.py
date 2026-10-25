"""Owned ELD packets and independent normative-window/cosine PCM; offline."""
import json,math,re,struct
from functools import lru_cache
from pathlib import Path
from generate_aac_main_tools_fixtures import sc,Noise,f32
from generate_aac_hcr_fixtures import cw,payload as hcr_payload
from generate_aac_rvlc_fixtures import scales as rvlc_scales
from generate_aac_ld_layout_fixtures import LAYOUTS,geometry
from generate_he_aac_packet_fixtures import field,frequency,packed,video_fixture
from generate_ffv1_gray_fixtures import element,uint
ROOT=Path(__file__).resolve().parents[1]
OUT=ROOT/'tests/fixtures/playback-errors/aac-eld'
@lru_cache(None)
def window(n):
    s=(ROOT/'crates/fvid-media/src/owned_aac/aac_eld_windows.rs').read_text()
    m=re.search(rf'WINDOW_{n}:.*?= \[(.*?)\];',s,re.S)
    result=list(map(float,re.findall(r'-?\d+\.\d+',m[1])))
    assert len(result)==4*n
    return result
@lru_cache(None)
def cosines(n,bins):
    return [[math.cos(math.pi/n*(i+.5-n/2)*(k+.5))for k in bins]for i in range(4*n)]
def asc(n,configuration,rate,flags=0):
    return packed(field(31,5)+field(7,6)+frequency(rate)+field(configuration,4)+field(n==480,1)+field(flags,3)+'0'+'0000'+'00')
def body(books,values,bins,scales,common,flags,tns=None,tns_bands=31):
    # ELD omits tags, pulse, gain-control, window shape, common_window and LTP.
    section=''.join(field(b,5 if flags&4 else 4)+(''if flags&4 and(b==11 or b>=16)else field(1,5))for b in books)
    if flags&2:sf,rv,_=rvlc_scales(0,books,scales,gain=140)
    else:
        sf='';rv='';ordinary=140;noise=50;intensity=0;first_noise=True
        for b,value in zip(books,scales):
            if b==0:continue
            if b==13:
                sf+=field(value-noise+256,9)if first_noise else sc(value-noise)
                noise=value;first_noise=False
            elif b in [14,15]:sf+=sc(value-intensity);intensity=value
            else:sf+=sc(value-ordinary);ordinary=value
    q=[values.get(k,0)for k in range(8)]
    if flags&1:
        data,hint,_=hcr_payload(0,[books],[1],[q]);spectral_header=field(len(data),14)+field(hint,6)
    else:
        spectral_header='';data=''
        for band,b in enumerate(books):
            if b in [0,13,14,15]:continue
            width=bins[band+1]-bins[band];dim=4 if b<=4 else 2
            for k in range(bins[band],bins[band]+width,dim):data+=cw(b,[values.get(i,0)for i in range(k,k+dim)])
    tns_bits=''if tns is None else field(1,2)+'0'+field(tns_bands,6)+field(1,5)+field(tns,1)+'0'+'001'
    return field(140,8)+(''if common else field(len(books),6))+section+sf+field(tns is not None,1)+tns_bits+spectral_header+rv+data
def mkv(config,packets,rate,n,channels):
    audio=element(0xe1,element(0xb5,struct.pack('>d',float(rate)))+uint(0x9f,channels))
    entry=uint(0xd7,1)+uint(0x83,2)+element(0x86,b'A_AAC')+element(0x63a2,config)+uint(0x23e383,n*1000000000//rate)+audio
    clusters=b''.join(element(0x1f43b675,uint(0xe7,i*n*1000000000//rate)+element(0xa3,b'\x81\0\0\x80'+p))for i,p in enumerate(packets))
    return element(0x1a45dfa3,uint(0x4286,1)+element(0x4282,b'matroska'))+element(0x18538067,element(0x1549a966,uint(0x2ad7b1,1))+element(0x1654ae6b,element(0xae,entry))+clusters)
def specifications():
    specs=[(n,24000,c,'base',0,0,False)for n in [512,480]for c in [1,2]]
    specs += [(n,48000,c,'layout',0,0,False)for n in [512,480]for c in LAYOUTS if c not in [1,2]]
    rates=[96000,88200,64000,48000,44100,32000,22050,16000,12000,11025,8000,7350,27713,37566,46009,12345]
    specs += [(512,r,1,'high',0,0,True)for r in rates]
    specs += [(n,24000,2,'resilience',f,1+(f%2),False)for n in [512,480]for f in range(1,8)]
    specs += [(n,24000,2,'ms-all',0,2,False)for n in [512,480]]
    specs += [(n,24000,2,t,0,1,False)for n in [512,480]for t in ['pns','correlated-pns','intensity','tns']]
    return specs

def generate():
    files={};cases=[];blob=bytearray()
    for n,rate,configuration,tool,flags,ms,high in specifications():
        elements,mapping,mask=LAYOUTS[configuration];channels=len(mapping);w=window(n)
        overlap=[[0.]*(3*n)for _ in mapping];noise=Noise();pcm=[];rows=[];packets=[]
        count,last=geometry(n,max(22050,min(48000,rate)))if high else(2,4)
        offsets=[0,4]if count==2 else [0]+[4*i for i in range(1,count)]+[n]
        # Only the first and final bands carry coefficients. Their exact last
        # lower boundary comes from the separately authored LD geometry facts.
        if high:offsets=[0,4]+list(range(8,8+4*(count-2),4))+[last,n]
        if len(offsets)!=count+1:
            offsets=[0]+[4*i for i in range(1,count-1)]+[last,n]
        if not high:offsets=[0,4,8]
        positions=(0,1,2,3,last,last+1,last+2,last+3)
        cos=cosines(n,positions)
        for frame in range(12):
            bits='';spectra=[];ch=0
            for element in elements:
                width=2 if element==1 else 1
                band_count=1 if element==3 else count
                books=[];scales=[];qs=[];values=[];signs=[]
                for c in range(width):
                    b=([1]if element==3 else([1,1]if not high else[1]+[0]*(count-2)+[1]))
                    scale=[140]*len(b)
                    q=[(-1,0,1)[(frame+k+ch+c)%3]for k in range(8)]
                    if frame>=7:q=[0]*8
                    if element==3:q[4:]=[0]*4
                    if flags&4:b[0]=17;q[:4]=[v*17 for v in q[:4]]
                    if width==2 and 2<=frame<=6:
                        if tool=='pns'and c==1:b[1]=13;scale[1]=54
                        if tool=='correlated-pns':b[1]=13;scale[1]=50+4*c
                        if tool=='intensity'and c==1:b[1]=14+frame%2;scale[1]=(-4,4)[frame%2]
                    books.append(b);scales.append(scale);qs.append(q)
                    value=dict(zip(positions,q))
                    # ELD resilience exercises RVLC escape deltas as well as HCR.
                    if flags&2:scale[0]=140+(-8,7,8,-7)[(frame+c)%4]
                    values.append(value)
                mask_bits=[bool((frame+b)%2)for b in range(band_count)]if ms==1 else[ms==2]*band_count
                if tool=='correlated-pns'and len(mask_bits)>1:mask_bits[1]=True
                if width==2:bits+=field(band_count,6)+field(ms,2)+(''.join(field(v,1)for v in mask_bits)if ms==1 else'')
                for c in range(width):
                    tns=(frame+c)%2 if tool=='tns'and frame<7 else None
                    bits+=body(books[c],values[c],offsets[:band_count+1],scales[c],width==2,flags,tns,geometry(n,max(22050,min(48000,rate)))[0])
                    spectrum={}
                    for band,book in enumerate(books[c]):
                        lo,hi=offsets[band:band+2];value=scales[c][band]
                        if book==13:
                            band_noise=noise.band(value);assert hi-lo==4
                            spectrum.update(zip(range(lo,hi),band_noise))
                        elif book not in [0,14,15]:
                            for k in range(lo,hi):
                                q=values[c].get(k,0);spectrum[k]=f32(math.copysign(abs(q)**(4/3),q)*2**((value-100)/4))if q else 0.
                    signs.append(tns);spectra.append(spectrum)
                if width==2:
                    left,right=spectra[-2:]
                    for band in range(band_count):
                        enabled=mask_bits[band];lb,rb=books[0][band],books[1][band]
                        for k in range(offsets[band],offsets[band+1]):
                            if rb in [14,15]:right[k]=f32(left.get(k,0.)*(1 if (rb==15)!=(ms==1 and enabled)else-1)*2**(-scales[1][band]/4))
                            elif lb==13 or rb==13:
                                if lb==rb==13 and enabled:right[k]=f32(left[k]*2**((scales[1][band]-scales[0][band])/4))
                            elif enabled:
                                a,b=left.get(k,0.),right.get(k,0.);left[k]=f32(a+b);right[k]=f32(a-b)
                for c in range(width):
                    direction=signs[c]
                    if direction is not None:
                        spectrum=spectra[-width+c];prior=0.
                        for k in (range(7,-1,-1)if direction else range(8)):
                            v=spectrum.get(k,0.)-math.sin(math.pi/7)*prior;spectrum[k]=f32(v);prior=v
                ch+=width
            payload=packed(bits);packets.append(payload);rows.append(dict(offset=len(blob),bytes=len(payload)));blob.extend(payload)
            outputs=[]
            for c,spectrum in enumerate(spectra):
                values=[spectrum.get(k,0.)for k in positions]
                transformed=[-sum(v*x for v,x in zip(values,row))/n for row in cos]
                accumulated=[x*w[i]+(overlap[c][i]if i<3*n else 0.)for i,x in enumerate(transformed)]
                outputs.append(accumulated[:n]);overlap[c]=accumulated[n:]
            for i in range(n):
                row=[0.]*channels
                for c,target in enumerate(mapping):row[target]=outputs[c][i]/32768
                pcm.append(struct.pack('<'+'f'*channels,*row))
        name=f'n{n}-c{configuration}'if tool=='base'else f'n{n}-r{rate}-c{configuration}-{tool}-f{flags}'
        config=asc(n,configuration,rate,flags);files[name+'.asc']=config;files[name+'.f32le']=b''.join(pcm)
        case=dict(name=name,n=n,rate=rate,configuration=configuration,mask=mask,tool=tool,flags=flags,asc=config.hex(),channels=channels,frames=rows,container_rate=rate,container_frame_samples=n,samples=12*n,pcm_offset=0,slots=n//64,bands=32)
        movie=video_fixture([case],blob,channels,filename=name+'.mp4',write=False)
        files[name+'.mp4']=movie.pop('data');case['video']=name+'.mp4'
        files[name+'.mka']=mkv(config,packets,rate,n,channels);case['matroska']=name+'.mka'
        cases.append(case)
    files['packets.bin']=bytes(blob)
    files['manifest.json']=(json.dumps(dict(cases=cases,provenance='Own ELD header/entropy writer, fixed layout ordering, PNS/MS/intensity/TNS arithmetic and independent direct cosine + normative 4N window + three-block overlap. Nonzero/decay frames; no private media, external decoder, subprocess or network.'),indent=2)+'\n').encode()
    return files
if __name__=='__main__':
    OUT.mkdir(exist_ok=True)
    for name,data in generate().items():(OUT/name).write_bytes(data)
