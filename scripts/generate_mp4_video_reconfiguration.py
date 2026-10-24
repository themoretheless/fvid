"""Owned decoder setup/precision/metadata transitions with reordered MP4 PTS."""
import json,struct
from pathlib import Path
from generate_vp9_chroma import frame,mp4,atom,word
from generate_mp4_fragment_description_regressions import atoms
from generate_avc_pcm_formats import Writer
from generate_avc_colour_formats import avc_configuration as configuration
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/mp4-video-reconfiguration'

def avc(depth,index):
    b=Writer();b.ue(0);b.ue(2);b.ue(0);b.u(0,4);b.ue(0);b.u(0,4);b.u(0);b.u(0);b.se(0);b.ue(1)
    values=[(1<<(depth-1))+index*3+p for p in range(3)]
    for mb in range(2):
        b.ue(25);b.align()
        for p in range(3):
            for _ in range(256):b.u(values[p],depth)
    nal=b.nal(0x65)
    return word(len(nal))+nal,values

def entry(codec,config,width,height,config_kind,identity):
    e=bytearray(78);e[6:8]=struct.pack('>H',1);e[24:28]=struct.pack('>HH',width,height);e[40:42]=struct.pack('>H',1);e[74:78]=struct.pack('>HH',24,65535)
    return atom(codec,e+atom(config_kind,config)+atom(b'pasp',word(identity,1))+atom(b'colr',b'nclx'+struct.pack('>HHHB',1 if identity==1 else 9,1 if identity==1 else 16,1 if identity==1 else 9,0 if identity==1 else 128))+atom(b'ccll',struct.pack('>HH',400 if identity==1 else 1000,200 if identity==1 else 500)))

def movie(packets,entries,selections,pts,fragmented=False):
    n=len(packets);base=dict(atoms(mp4(packets,8,'420',(32,16),list(range(1,n+1)))))
    offset=len(atom(b'ftyp',base[b'ftyp']))+8;offsets=[]
    for p in packets:offsets.append(offset);offset+=len(p)
    def rewrite(data):
        out=b''
        for k,b in atoms(data):
            if k in [b'trak',b'mdia',b'minf',b'stbl']:b=rewrite(b)
            elif k==b'tkhd':b=bytearray(b);b[76:84]=bytes(8)
            elif k==b'stsd':b=word(0,len(entries))+b''.join(entries)
            elif fragmented and k in [b'stts',b'stsc',b'stco',b'stss']:b=word(0,0)
            elif fragmented and k==b'stsz':b=word(0,0,0)
            elif k==b'stsc':b=word(0,n)+b''.join(word(i+1,1,d)for i,d in enumerate(selections))
            elif k==b'stco':b=word(0,n,*offsets)
            if k==b'stbl' and not fragmented:b+=atom(b'ctts',word(1<<24,n)+b''.join(struct.pack('>Ii',1,t-i)for i,t in enumerate(pts)))
            out+=atom(k,b)
        return out
    if not fragmented:return atom(b'ftyp',base[b'ftyp'])+atom(b'mdat',base[b'mdat'])+atom(b'moov',rewrite(base[b'moov']))
    init=atom(b'ftyp',b'iso5'+word(0)+b'iso5avc1vp09')+atom(b'moov',rewrite(base[b'moov'])+atom(b'mvex',atom(b'trex',word(0,1,1,1,0,0x02000000))))
    def fragment(i):
        h=atom(b'tfhd',word(0x02003a,1,selections[i],1,len(packets[i]),0x02000000))
        def build(offset):return atom(b'moof',atom(b'mfhd',word(0,i+1))+atom(b'traf',h+atom(b'tfdt',word(0,i))+atom(b'trun',word(0x01000801,1,offset)+struct.pack('>i',pts[i]-i))))
        m=build(0);return build(len(m)+8)+atom(b'mdat',packets[i])
    return init+b''.join(fragment(i)for i in range(n))

def generate():
    files={};cases=[];ids=[1,1,2,2,1];pts=[0,3,1,2,4]
    for kind in ['avc','codec-switch','constant']:
        ids=[1]*5 if kind=='constant' else [1,1,2,2,1]
        first=entry(b'avc1',configuration(3,8,8,False,vui=(False,1,1,1)),32,16,b'avcC',1)
        if kind in ['avc','constant']:second=entry(b'avc1',configuration(3,10,10,False,vui=(True,9,16,9)),32,16,b'avcC',2)
        else:
            source=dict(atoms(mp4([frame(10,'444',32,16)],10,'444',(32,16),[1])))
            def find(data,kind):
                for k,b in atoms(data):
                    if k==kind:return b
                    if k in [b'trak',b'mdia',b'minf',b'stbl']:
                        result=find(b,kind)
                        if result is not None:return result
            vp=list(atoms(find(source[b'moov'],b'stsd')[8:]))[0][1]
            config=dict(atoms(vp[78:]))[b'vpcC'];second=entry(b'vp09',config,32,16,b'vpcC',2)
        packets=[];records=[]
        for i,identity in enumerate(ids):
            depth=8 if identity==1 else 10
            if kind in ['avc','constant'] or identity==1:p,values=avc(depth,i)
            else:p=frame(10,'444',32,16);values=[512]*3
            packets.append(p);records.append(dict(depth=depth,values=values,description=identity,pts=pts[i]))
            files[f'{kind}-control-{i}.mp4']=movie([p],[first if identity==1 else second],[1],[0])
            files[f'{kind}-{i}.packet']=p
        for fragmented in [False,True]:files[kind+('-fragmented'if fragmented else'')+'.mp4']=movie(packets,[first,second],ids,pts,fragmented)
        for fragmented in [False,True]:files[kind+'-start-second'+('-fragmented'if fragmented else'')+'.mp4']=movie(packets,[second,first],[3-d for d in ids],pts,fragmented)
        if kind!='constant':files[kind+'-wrong-first-setup.mp4']=movie(packets,[first],[1]*5,pts)
        if kind=='avc':
            files['avc-rewind-tail.mp4']=movie(packets[:4],[first,second],ids[:4],pts[:4])
            invalid=entry(b'avc1',b'',32,16,b'avcC',3)
            files['avc-invalid-late-configuration.mp4']=movie(packets,[first,second,invalid],[1,1,2,2,3],list(range(5)))
        cases.append(dict(name=kind,frames=records,display_order=sorted(range(5),key=lambda i:pts[i])))
    # Unchanged PCM audio must not be rejected because the video setup changes.
    base=dict(atoms(files['avc.mp4']))
    pcm=b''.join(struct.pack('<hh',1000+i%31,-1000-i%31)for i in range(4800))
    files['constant-stereo.s16le']=pcm
    audio_offset=len(atom(b'ftyp',base[b'ftyp']))+8+len(base[b'mdat'])
    e=bytearray(28);e[6:8]=struct.pack('>H',1);e[16:20]=struct.pack('>HH',2,16);e[24:28]=word(48000<<16)
    stbl=atom(b'stbl',atom(b'stsd',word(0,1)+atom(b'sowt',e))+atom(b'stts',word(0,1,4800,1))+atom(b'stsz',word(0,4,4800))+atom(b'stsc',word(0,1,1,4800,1))+atom(b'stco',word(0,1,audio_offset)))
    dinf=atom(b'dinf',atom(b'dref',word(0,1)+atom(b'url ',word(1))))
    tkhd=bytearray(84);tkhd[:4]=word(3);tkhd[12:16]=word(2);tkhd[20:24]=word(5);tkhd[40:44]=tkhd[56:60]=word(65536);tkhd[72:76]=word(0x40000000)
    mdhd=bytearray(24);mdhd[12:20]=word(48000,4800)
    audio=atom(b'trak',atom(b'tkhd',tkhd)+atom(b'mdia',atom(b'mdhd',mdhd)+atom(b'hdlr',word(0,0)+b'soun'+bytes(12))+atom(b'minf',dinf+stbl)))
    files['avc-with-constant-audio.mp4']=atom(b'ftyp',base[b'ftyp'])+atom(b'mdat',base[b'mdat']+pcm)+atom(b'moov',base[b'moov']+audio)
    files['manifest.json']=(json.dumps(dict(cases=cases,provenance='Owned I_PCM AVC with repeated SPS/PPS IDs but changed depth, own VP9 10-bit 444 keyframes, independent exact constant-plane oracles, reordered PTS, alternating pasp/nclx/ccll metadata. No private media, encoder, FFmpeg or network.'),indent=2)+'\n').encode()
    return files

if __name__=='__main__':
    OUT.mkdir(parents=True,exist_ok=True)
    for name,data in generate().items():(OUT/name).write_bytes(data)
