"""Own AAC pulse spectra, scalar PCM oracles and MP4 setup transitions."""
import json,math,struct
from fractions import Fraction as F
from pathlib import Path
from generate_vp9_chroma import frame,mp4,atom,word
from generate_mp4_fragment_description_regressions import atoms
from generate_mp4_audio_selection_regressions import build_esds,pack_bits
from generate_mp4_variable_rate_edits import edit_movie
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/mp4-aac-transitions'
SCALE=14112000

def individual(gain,band,amplitude):
    return f'{gain:08b}'+'0'+'00'+'0'+'000001'+'0'+'0001'+'00001'+'0'+'1'+'00'+'000000'+f'{band:05b}{amplitude:04b}'+'0'+'0'+'0'
def packet(specs):
    bits=('001'+'0000'+'0' if len(specs)==2 else '000'+'0000')
    return pack_bits(bits+''.join(individual(*s) for s in specs)+'111')
def oracle(n,frames):
    overlap=[[0.0]*n for _ in frames[0]];out=bytearray()
    for specs in frames:
        planes=[]
        for c,(gain,band,amp) in enumerate(specs):
            value=-(amp*amp**(1/3)*2**((gain-100)/4))
            value=struct.unpack('<f',struct.pack('<f',value))[0]
            transformed=[value*2/n*math.cos(math.pi/n*(i+.5+n/2)*(band+.5))*math.sin(math.pi/(2*n)*(i+.5)) for i in range(2*n)]
            planes.append([(overlap[c][i]+transformed[i])/65536 for i in range(n)])
            overlap[c]=transformed[n:]
        for i in range(n):
            for plane in planes:out+=struct.pack('<f',plane[i])
    return bytes(out)
def generate(shapes=None,controls=None,asc_configs=None):
    shapes=shapes or [(48000,2,1024,2,3),(32000,1,960,1,5),(44100,2,1024,2,4)]
    entries=[];groups=[];files={}
    for index,(rate,ch,n,profile,rate_index) in enumerate(shapes):
        specs=[[(150+f*3+c,(f+c)%4,4+f+c) for c in range(ch)]for f in range(3)]
        encoded,pcm=controls[index] if controls is not None else ([packet(s) for s in specs],oracle(n,specs))
        groups.append(encoded);files[f'control-{index}.f32le']=pcm
        asc=pack_bits(f'{profile:05b}{rate_index:04b}{ch:04b}'+('1'if n==960 else'0')+'00')
        if asc_configs is not None:asc=asc_configs[index]
        e=bytearray(28);e[6:8]=struct.pack('>H',1);e[16:20]=struct.pack('>HH',ch,16);e[24:28]=word(rate<<16)
        entries.append(atom(b'mp4a',e+atom(b'esds',build_esds(asc))))
    video=dict(atoms(mp4([frame(8,'420',16,16)]*16,8,'420',(16,16),list(range(1,17)))))
    header=atom(b'ftyp',video[b'ftyp'])
    def movie(selection,order=None,fragmented=False):
        order=order or [0,1,2];raw=[];timing=[];ids=[];offsets=[];at=len(header)+8+len(video[b'mdat'])
        for i in selection:
            for p in groups[i]:offsets.append(at);at+=len(p);raw.append(p);timing.append(shapes[i][2]*(SCALE//shapes[i][0]));ids.append(order.index(i)+1)
        count=len(raw)
        stbl=atom(b'stbl',atom(b'stsd',word(0,len(order))+b''.join(entries[i]for i in order))
            +atom(b'stts',word(0,0)if fragmented else word(0,count)+b''.join(word(1,d)for d in timing))
            +atom(b'stsz',word(0,0,0)if fragmented else word(0,0,count,*map(len,raw)))
            +atom(b'stsc',word(0,0)if fragmented else word(0,count)+b''.join(word(j+1,1,d)for j,d in enumerate(ids)))
            +atom(b'stco',word(0,0)if fragmented else word(0,count,*offsets)))
        t=bytearray(84);t[:4]=word(3);t[12:16]=word(2);t[20:24]=word((sum(timing)*50+SCALE-1)//SCALE)
        t[40:44]=t[56:60]=word(65536);t[72:76]=word(0x40000000)
        mdhd=bytearray(24);mdhd[12:20]=word(SCALE,sum(timing))
        track=atom(b'trak',atom(b'tkhd',t)+atom(b'mdia',atom(b'mdhd',mdhd)+atom(b'hdlr',word(0,0)+b'soun'+bytes(12))+atom(b'minf',atom(b'dinf',atom(b'dref',word(0,1)+atom(b'url ',word(1))))+stbl)))
        if not fragmented:return header+atom(b'mdat',video[b'mdat']+b''.join(raw))+atom(b'moov',video[b'moov']+track)
        brand=bytearray(video[b'ftyp']);brand[:4]=b'iso5';brand[8:12]=b'iso5'
        result=atom(b'ftyp',brand)+atom(b'mdat',video[b'mdat'])+atom(b'moov',video[b'moov']+track+atom(b'mvex',atom(b'trex',word(0,2,1,0,0,0x02000000))))
        dts=0
        for j,(p,d,identity)in enumerate(zip(raw,timing,ids)):
            tfhd=atom(b'tfhd',word(0x02003a,2,identity,d,len(p),0x02000000))
            def moof(offset):return atom(b'moof',atom(b'mfhd',word(0,j+1))+atom(b'traf',tfhd+atom(b'tfdt',word(0,dts))+atom(b'trun',word(1,1,offset))))
            m=moof(0);result+=moof(len(m)+8)+atom(b'mdat',p);dts+=d
        return result
    for i in range(3):files[f'control-{i}.mp4']=movie([i],[i])
    files['changes.mp4']=movie([0,1,2,0]);files['third.mp4']=movie([0,1,2,0],[2,1,0])
    files['fragmented.mp4']=movie([0,1,2,0],fragmented=True)
    def equivalent(body,audio=False):
        out=b''
        for k,b in atoms(body):
            if k==b'trak':b=equivalent(b,b'soun' in b)
            elif k in [b'moov',b'mdia',b'minf',b'stbl']:b=equivalent(b,audio)
            elif audio and k==b'stsc':b=bytearray(b);b[28:32]=word(2)
            out+=atom(k,b)
        return out
    files['equivalent.mp4']=equivalent(movie([0,1,2,0],[0,0,1,2]))
    for source in ['changes','third','fragmented']:
        files[source+'-edits.mp4']=edit_movie(files[source+'.mp4'],[(5,-1),(160,SCALE//50),(7,-1),(160,SCALE//50)])
        files[source+'-fractional.mp4']=edit_movie(files[source+'.mp4'],[(1,-1),(400,SCALE//50),(2,-1),(400,SCALE//50)],clock=3000)
    def part(control,index,first,last,pts):
        rate,ch,n,_,_=shapes[control]
        return dict(control=control,index=index,first=first,last=last,rate=rate,channels=ch,pts=[pts.numerator,pts.denominator])
    def silence(frames,pts):return dict(control=None,first=0,last=frames,rate=48000,channels=2,pts=[pts.numerator,pts.denominator])
    continuous=[];at=F(0)
    for identity in [0,1,2,0]:
        rate,ch,n,_,_=shapes[identity]
        for i in range(3):continuous.append(part(identity,i,0,n,at+F(i*n,rate)))
        at+=F(3*n,rate)
    cuts=[part(0,0,960,1024,F(5,1000)),part(0,1,0,1024,F(19,3000)),part(0,2,0,1024,F(83,3000)),
        part(1,0,0,960,F(49,1000)),part(1,1,0,960,F(79,1000)),part(1,2,0,960,F(109,1000)),
        part(2,0,0,1024,F(139,1000)),part(2,1,0,123,F(139,1000)+F(1024,44100))]
    fractional=[part(0,0,960,1024,F(1,3000)),part(0,1,0,1024,F(5,3000)),part(0,2,0,1024,F(69,3000)),
        part(1,0,0,960,F(133,3000)),part(1,1,0,960,F(223,3000)),part(1,2,0,939,F(313,3000))]
    def shifted(parts,shift):
        result=[]
        for p in parts:
            p=dict(p);t=F(*p['pts'])+shift;p['pts']=[t.numerator,t.denominator];result.append(p)
        return result
    edited=[silence(240,F(0))]+cuts+[silence(335,F(165,1000)+F(1,48000))]+shifted(cuts,F(167,1000))
    fraction=[silence(16,F(0))]+fractional+[silence(31,F(401,3000)+F(1,48000))]+shifted(fractional,F(134,1000))
    cases=[]
    for name in ['changes','third','fragmented','equivalent']:
        cases.append(dict(name=name,parts=continuous,duration=[at.numerator,at.denominator]))
        if name=='equivalent':continue
        cases.append(dict(name=name+'-edits',parts=edited,duration=[332,1000]))
        cases.append(dict(name=name+'-fractional',parts=fraction,duration=[803,3000]))
    files['disjoint.mp4']=edit_movie(files['changes.mp4'],[(20,SCALE*80//1000),(10,0),(3,-1)])
    cases.append(dict(name='disjoint',parts=[part(1,0,512,960,F(0)),part(1,1,0,192,F(14,1000)),part(0,0,0,480,F(20,1000)),silence(144,F(30,1000))],duration=[33,1000]))
    files['manifest.json']=(json.dumps(dict(cases=cases,scale=SCALE,shapes=[dict(rate=r,channels=c,frames=n,profile=p)for r,c,n,p,_ in shapes],selection=[0,1,2,0],provenance='Owned AAC pulse spectra with analytic scalar IMDCT+overlap float PCM, LC1024 stereo48k/Main960 mono32k/LC1024 stereo44.1k, neutral VP9. No private media, external codec, FFmpeg or network.'),indent=2)+'\n').encode()
    return files
if __name__=='__main__':
    OUT.mkdir(parents=True,exist_ok=True)
    for name,data in generate().items():(OUT/name).write_bytes(data)
