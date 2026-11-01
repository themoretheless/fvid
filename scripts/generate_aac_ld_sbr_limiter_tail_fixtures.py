"""Own short LD-SBR discarded-tail limiter regression; explicit offline generation.

Direct full cosine/window convolution, independent CLDFB superposition and
Decimal limiter thresholds. No source media/parameter sets, native/foreign
decoder, process or network supplies the expected PCM.
"""
import json, math, struct
from decimal import Decimal as D, localcontext
from pathlib import Path
from generate_aac_eld_fixtures import body, window as core_window, mkv
from generate_aac_eld_sbr_fixtures import analysis
from generate_aac_eld_cldfb_fixtures import window, synthesis
from generate_aac_sbr_dsp_fixtures import header as base_header
from generate_aac_sbr_extension_fixtures import crc
from generate_aac_sbr_history_fixtures import row
from generate_aac_sbr_frequency_oracles import tables
from generate_he_aac_packet_fixtures import field, frequency, packed, video_fixture
ROOT=Path(__file__).resolve().parents[1]
FIX=ROOT/'tests/fixtures/playback-errors'
OUT=FIX/'aac-ld-sbr-limiter-tail'


def limiter_borders(mode):
    if mode==0:return [10,20]
    # One retained patch maps source2..10 to target10..18. The discarded
    # two-band tail18..20 is still in the SBR synthesis range, but18 is not
    # an internal boundary between two retained patches.
    protected={10,20};values=list(range(10,21,2));cursor=1
    with localcontext() as ctx:
        ctx.prec=80
        threshold=D(2)**(D('.49')/[D('1.2'),D(2),D(3)][mode-1])
        while cursor<len(values):
            a,b=values[cursor-1:cursor+1]
            if D(b)/D(a)>=threshold:cursor+=1
            elif b not in protected:values.pop(cursor)
            elif a not in protected:values.pop(cursor-1)
            else:cursor+=1
    return values


def core(n):
    w=core_window(n);coef=2**16;bin=64
    impulse=[-coef*math.cos(math.pi/n*(t+.5-n/2)*(bin+.5))*w[t]/n/32768 for t in range(4*n)]
    pcm=[sum((1 if f%3 else -1)*impulse[t-f*n] for f in range(8)if 0<=t-f*n<4*n)
         for t in range(n//4,8*n+n//4)]
    return [v[0]for v in struct.iter_unpack('<f',b''.join(struct.pack('<f',v)for v in pcm))]


def levels(energy,borders):
    target=2**24;ratio=.5
    gain=[math.sqrt(target/(1+ratio)/(1+e))for e in energy]
    noise=[math.sqrt(target*ratio/(1+ratio))]*10
    for lo,hi in zip(borders,borders[1:]):
        a,b=lo-10,hi-10
        maximum=min(math.sqrt(((b-a)*target+1e-12)/(sum(energy[a:b])+1e-12))*1.41254,1e5)
        for i in range(a,b):
            if gain[i]>maximum:noise[i]*=maximum/gain[i];gain[i]=maximum
        reconstructed=sum(energy[i]*gain[i]**2+noise[i]**2 for i in range(a,b))
        boost=min(math.sqrt(((b-a)*target+1e-12)/(reconstructed+1e-12)),1.584893192)
        for i in range(a,b):gain[i]*=boost;noise[i]*=boost
    return gain,noise


def main():
    OUT.mkdir(exist_ok=True);c=window();noise_table=[complex(*v)for v in struct.iter_unpack('<dd',(FIX/'aac-sbr-noise-protocol.f64le').read_bytes())]
    _,high,_,_=tables(10,20,0,False,0,0)
    offsets=[0,4,8,12,16,20,24,28,32,36,40,44,52,60,68]
    files={};cases=[];blob=bytearray()
    for n in [480,512]:
        slots=n//32;low=analysis(core(n),c)
        for mode in range(4):
            borders=limiter_borders(mode);rows=[]
            for start in range(0,len(low),slots):
                current=low[start:start+slots]
                hf=[[v[k]for k in range(2,10)]+[0j,0j]for v in current]
                energy=[sum(abs(v[k])**2 for v in hf)/slots for k in range(10)]
                gain,random=levels(energy,borders)
                for j,(lf,h)in enumerate(zip(current,hf)):
                    t=start+j
                    adjusted=[h[k]*gain[k]+noise_table[(t*10+k+1)%512]*random[k]for k in range(10)]
                    rows.append(lf+adjusted+[0j]*44)
            gold=b''.join(struct.pack('<f',v/32768)for v in synthesis(rows,64,c))
            h=base_header(mode,True);h=h[:5]+field(14,4)+h[9:]
            asc=packed(field(31,5)+field(7,6)+frequency(24000)+field(1,4)+field(n==480,1)+'000111'+h+'000000')
            packets=[];frames=[]
            for f in range(8):
                prefix=body([0]*13+[1],{64:1 if f%3 else -1},offsets,[164]*14,False,0)
                temporal=f>0
                env=row([0]*(len(high)-1)if temporal else[36]+[0]*(len(high)-2),temporal,False,False,False)
                qn=row([0 if temporal else 7],temporal,False,False,True)
                data='00'+'00001'+field(temporal,1)*2+'00'+env+qn+'00'
                wire=packed(prefix+field(crc(data),10)+data);frames.append(dict(offset=len(blob),bytes=len(wire)));blob.extend(wire);packets.append(wire)
            name=f'n{n}-lim{mode}'
            case=dict(name=name,n=n,rate=48000,channels=1,slots=slots,bands=64,asc=asc.hex(),frames=frames,
                      samples=16*n,out_n=2*n,pcm_offset=0,container_rate=48000,container_frame_samples=2*n,
                      video=name+'.mp4',file=name+'.mkv',golden=name+'.f32le',limiter_mode=mode,
                      expected_borders=borders,edges=[2*n,6*n,14*n])
            files[case['golden']]=gold;files[case['video']]=video_fixture([case],blob,1,write=False)['data'];files[case['file']]=mkv(asc,packets,48000,2*n,1);cases.append(case)
    files['packets.bin']=bytes(blob)
    files['manifest.json']=(json.dumps(dict(cases=cases,exports=[dict(c,name=c['name']+'-matroska',video=c['file'])for c in cases],scope='Own480/512 eight-AU mono dual-rate,all4 limiter densities,one retained source2->target10 width8 patch and discarded2-band tail. Independent direct transform/convolution and complete synthesis-range limiter partition. No private/ISO media or parameter sets,native/foreign decoder,process or network.'),indent=2)+'\n').encode()
    for name,data in files.items():(OUT/name).write_bytes(data)
    print('8 programmes;8 MP4 videos+8 Matroska audio controls')


if __name__=='__main__':main()
