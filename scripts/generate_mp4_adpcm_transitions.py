"""Owned analytic ADPCM blocks with distinct MP4 duration clocks."""
import json,struct
from pathlib import Path
from fractions import Fraction as F
from generate_vp9_chroma import frame,mp4,atom,word
from generate_mp4_fragment_description_regressions import atoms
from generate_mp4_variable_rate_edits import edit_movie
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/mp4-adpcm-transitions'

def generate():
    shapes=[(b'ima4',8000,1,64,1),(b'ms\0\x11',16000,2,9,8),(b'ms\0\x02',8000,1,18,2)]
    entries=[];groups=[];goldens=[];files={}
    coefficients=[(256,0),(512,-256),(0,0),(192,64),(240,0),(460,-208),(392,-232)]
    for identity,(codec,rate,ch,n,multiplier)in enumerate(shapes):
        e=bytearray(28);e[6:8]=struct.pack('>H',1);e[16:20]=struct.pack('>HH',ch,16);e[24:28]=word(rate<<16)
        block_bytes=[34,16,15][identity]
        if identity==1:fmt=struct.pack('<HHIIHHHH',17,ch,rate,rate*16//9,16,4,2,9)
        elif identity==2:fmt=struct.pack('<HHIIHHHHH',2,ch,rate,rate*15//18,15,4,32,18,7)+b''.join(struct.pack('<hh',a,b)for a,b in coefficients)
        else:fmt=None
        entries.append(atom(codec,e+(atom(b'wave',atom(codec,fmt))if fmt else b'')))
        encoded=[];pcm=[]
        for packet_index,count in enumerate([1,2,1]):
            raw=b'';expected=b''
            for j in range(count*multiplier):
                predictor=(packet_index*8+j+1)*128
                if identity==0:
                    raw+=struct.pack('>H',predictor)+bytes([0x11])*32
                    values=[(predictor+k)/32768 for k in range(1,65)]
                elif identity==1:
                    raw+=struct.pack('<hBBhBB',predictor,0,0,-predictor,0,0)+bytes(8)
                    values=[v/32768 for _ in range(9)for v in [predictor,-predictor]]
                else:
                    raw+=bytes([0])+struct.pack('<hhh',16,predictor,predictor)+bytes(8)
                    values=[predictor/32768]*18
                expected+=struct.pack('<'+'f'*len(values),*values)
            encoded.append(raw);pcm.append(expected);files[f'pcm-{identity}-{packet_index}.f32le']=expected
        groups.append(encoded);goldens.append(pcm)
    video=dict(atoms(mp4([frame(8,'420',16,16)]*6,8,'420',(16,16),list(range(1,7)))))
    header=atom(b'ftyp',video[b'ftyp'])
    def movie(scale,fragmented=False,trim=False,order=None,selection=None):
        selection=selection or [0,1,2,0]
        order=order or [0,1,2];raw=[];durations=[];ids=[];offsets=[];at=len(header)+8+len(video[b'mdat'])
        for identity in selection:
            codec,rate,ch,n,multiplier=shapes[identity]
            for j,p in enumerate(groups[identity]):
                frames=n*multiplier*[1,2,1][j]
                if trim and identity==0 and j==2:frames-=16
                assert frames*scale%rate==0
                raw.append(p);durations.append(frames*scale//rate);ids.append(order.index(identity)+1);offsets.append(at);at+=len(p)
        count=len(raw)
        stbl=atom(b'stbl',atom(b'stsd',word(0,len(order))+b''.join(entries[i]for i in order))
            +atom(b'stts',word(0,0)if fragmented else word(0,count)+b''.join(word(1,d)for d in durations))
            +atom(b'stsz',word(0,0,0)if fragmented else word(0,0,count,*map(len,raw)))
            +atom(b'stsc',word(0,0)if fragmented else word(0,count)+b''.join(word(j+1,1,d)for j,d in enumerate(ids)))
            +atom(b'stco',word(0,0)if fragmented else word(0,count,*offsets)))
        t=bytearray(84);t[:4]=word(3);t[12:16]=word(2);t[20:24]=word((sum(durations)*50+scale-1)//scale)
        t[40:44]=t[56:60]=word(65536);t[72:76]=word(0x40000000)
        mdhd=bytearray(24);mdhd[12:20]=word(scale,sum(durations))
        track=atom(b'trak',atom(b'tkhd',t)+atom(b'mdia',atom(b'mdhd',mdhd)+atom(b'hdlr',word(0,0)+b'soun'+bytes(12))+atom(b'minf',atom(b'dinf',atom(b'dref',word(0,1)+atom(b'url ',word(1))))+stbl)))
        if not fragmented:return header+atom(b'mdat',video[b'mdat']+b''.join(raw))+atom(b'moov',video[b'moov']+track)
        brand=bytearray(video[b'ftyp']);brand[:4]=b'iso5';brand[8:12]=b'iso5'
        result=atom(b'ftyp',brand)+atom(b'mdat',video[b'mdat'])+atom(b'moov',video[b'moov']+track+atom(b'mvex',atom(b'trex',word(0,2,1,0,0,0x02000000))))
        dts=0
        for j,(p,d,identity)in enumerate(zip(raw,durations,ids)):
            tfhd=atom(b'tfhd',word(0x02003a,2,identity,d,len(p),0x02000000))
            def moof(offset):return atom(b'moof',atom(b'mfhd',word(0,j+1))+atom(b'traf',tfhd+atom(b'tfdt',word(0,dts))+atom(b'trun',word(1,1,offset))))
            m=moof(0);result+=moof(len(m)+8)+atom(b'mdat',p);dts+=d
        return result
    cases=[]
    for scale in [2000,48000,96000]:
        for variant in ['regular','fragmented','trim','third']:
            name=f'{variant}-{scale}'
            files[name+'.mp4']=movie(scale,fragmented=variant=='fragmented',trim=variant=='trim',order=[2,1,0]if variant=='third'else None)
            parts=[];at=F(0)
            for identity in [0,1,2,0]:
                _,rate,ch,n,multiplier=shapes[identity]
                for j,count in enumerate([1,2,1]):
                    kept=n*multiplier*count-(16 if variant=='trim'and identity==0 and j==2 else 0)
                    parts.append(dict(oracle=f'pcm-{identity}-{j}.f32le',rate=rate,channels=ch,first=0,last=kept,pts=[at.numerator,at.denominator]));at+=F(kept,rate)
            cases.append(dict(name=name,scale=scale,parts=parts,duration=[at.numerator,at.denominator]))
    for identity in range(3):
        name=f'control-{identity}';files[name+'.mp4']=movie(2000,order=[identity],selection=[identity]);parts=[];at=F(0)
        _,rate,ch,n,multiplier=shapes[identity]
        for j,count in enumerate([1,2,1]):
            kept=n*multiplier*count;parts.append(dict(oracle=f'pcm-{identity}-{j}.f32le',rate=rate,channels=ch,first=0,last=kept,pts=[at.numerator,at.denominator]));at+=F(kept,rate)
        cases.append(dict(name=name,scale=2000,parts=parts,duration=[at.numerator,at.denominator]))
    def cut(identity,j,first,last,pts):
        _,rate,ch,_,_=shapes[identity]
        return dict(oracle=f'pcm-{identity}-{j}.f32le',rate=rate,channels=ch,first=first,last=last,pts=[pts.numerator,pts.denominator])
    def silence(frames,pts):return dict(oracle=None,rate=8000,channels=1,first=0,last=frames,pts=[pts.numerator,pts.denominator])
    cuts=[cut(0,0,32,64,F(2,1000)),cut(0,1,0,128,F(6,1000)),cut(0,2,0,64,F(22,1000)),cut(1,0,0,32,F(30,1000))]
    repeat=[]
    for p in cuts:
        p=dict(p);t=F(*p['pts'])+F(33,1000);p['pts']=[t.numerator,t.denominator];repeat.append(p)
    parts=[silence(16,F(0))]+cuts+[silence(24,F(32,1000))]+repeat
    for scale in [2000,48000,96000]:
        for variant in ['regular','fragmented','third']:
            source=f'{variant}-{scale}';name=source+'-edits'
            files[name+'.mp4']=edit_movie(files[source+'.mp4'],[(2,-1),(30,scale*4//1000),(3,-1),(30,scale*4//1000)])
            cases.append(dict(name=name,scale=scale,parts=parts,duration=[65,1000]))
    files['manifest.json']=(json.dumps(dict(cases=cases,provenance='Owned IMA4 monotonic ramp; IMA WAV stereo signed predictor plateaus; MS ADPCM mono predictor plateaus. Grouped blocks, rate/channels/coding changes, three duration clocks, intentional final-block trim, neutral VP9. No private media, external codec, FFmpeg or network.'),indent=2)+'\n').encode()
    return files
if __name__=='__main__':
    OUT.mkdir(parents=True,exist_ok=True)
    for n,b in generate().items():(OUT/n).write_bytes(b)
