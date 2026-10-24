"""Own PCM coding/rate/channel transitions beside synthetic VP9, without FFmpeg."""
import json,struct
from pathlib import Path
from generate_vp9_chroma import frame,mp4,atom,word
from generate_mp4_fragment_description_regressions import atoms
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/mp4-audio-reconfiguration'
def generate(pcm_shapes=None,pcm_only=False,pcm_frames=None,pcm_values=None):
    files={};ids=[1,2,3,1,2,3];shapes=pcm_shapes or [(b'sowt',48000,2,16),(b'fl32',32000,1,32),(b'in24',44100,2,24)];entries=[];packets=[];records=[]
    scale=14112000 if pcm_frames else 1000000
    durations=[]
    for codec,rate,channels,depth in shapes:
        e=bytearray(28);e[6:8]=struct.pack('>H',1);e[16:20]=struct.pack('>HH',channels,depth);e[24:28]=word(rate<<16)
        entries.append(atom(codec,e+(atom(b'enda',b'\0\1')if codec!=b'sowt'else b'')))
    for i,identity in enumerate(ids):
        codec,rate,channels,depth=shapes[identity-1];count=pcm_frames[i] if pcm_frames else rate//50;raw=bytearray();golden=bytearray()
        for n in range(count):
            for c in range(channels):
                sign=-1 if c else 1
                if pcm_values is not None:
                    value=pcm_values[identity-1][c]
                    if codec==b'fl32':raw+=struct.pack('<f',value)
                    elif codec==b'fl64':raw+=struct.pack('<d',value)
                    else:raw+=int(round(value*(1<<(depth-1)))).to_bytes(depth//8,'little',signed=True)
                elif codec==b'fl32':value=((n%17)-8)/32;raw+=struct.pack('<f',value)
                else:
                    sample=sign*((1000 if depth==16 else 100000)+n%31+i*7);raw+=sample.to_bytes(depth//8,'little',signed=True);value=sample/(1<<(depth-1))
                golden+=struct.pack('<f',value)
        packets.append(bytes(raw));files[f'pcm-{i}.f32le']=bytes(golden);records.append(dict(description=identity,rate=rate,channels=channels,depth=depth,pts=sum(durations),frames=count,oracle=f'pcm-{i}.f32le'))
        assert count*scale%rate==0
        durations.append(count*scale//rate)
    base=dict(atoms(mp4([frame(8,'420',16,16)]*6,8,'420',(16,16),list(range(1,7)))))
    ftyp=atom(b'ftyp',base[b'ftyp']);offset=len(ftyp)+8+len(base[b'mdat']);offsets=[]
    for p in packets:offsets.append(offset);offset+=len(p)
    def audio(fragmented=False,edits=False):
        stbl=atom(b'stbl',atom(b'stsd',word(0,3)+b''.join(entries))+atom(b'stts',word(0,0)if fragmented else (word(0,1,6,20000) if pcm_frames is None else word(0,6)+b''.join(word(1,d) for d in durations)))+atom(b'stsz',word(0,0,0)if fragmented else word(0,0,6,*map(len,packets)))+atom(b'stsc',word(0,0)if fragmented else word(0,6)+b''.join(word(i+1,1,d)for i,d in enumerate(ids)))+atom(b'stco',word(0,0)if fragmented else word(0,6,*offsets)))
        dinf=atom(b'dinf',atom(b'dref',word(0,1)+atom(b'url ',word(1))))
        tkhd=bytearray(84);tkhd[:4]=word(3);tkhd[12:16]=word(2);tkhd[20:24]=word(7 if edits else (sum(durations)*50+scale-1)//scale);tkhd[40:44]=tkhd[56:60]=word(65536);tkhd[72:76]=word(0x40000000)
        mdhd=bytearray(24);mdhd[12:20]=word(scale,sum(durations))
        extra=atom(b'edts',atom(b'elst',word(0,2)+struct.pack('>IiI',1,-1,65536)+struct.pack('>IiI',6,0,65536)))if edits else b''
        return atom(b'trak',atom(b'tkhd',tkhd)+extra+atom(b'mdia',atom(b'mdhd',mdhd)+atom(b'hdlr',word(0,0)+b'soun'+bytes(12))+atom(b'minf',dinf+stbl)))
    files['pcm.mp4']=ftyp+atom(b'mdat',base[b'mdat']+b''.join(packets))+atom(b'moov',base[b'moov']+audio())
    files['pcm-edits.mp4']=ftyp+atom(b'mdat',base[b'mdat']+b''.join(packets))+atom(b'moov',base[b'moov']+audio(edits=True))
    # Retain the complete initial video in stbl; append only fragmented audio.
    # default-base-is-moof is advertised by the iso5 compatible brand.
    fragment_header=atom(b'ftyp',b'iso5'+word(0)+b'iso5vp09');assert len(fragment_header)==len(ftyp)
    init=fragment_header+atom(b'mdat',base[b'mdat'])+atom(b'moov',base[b'moov']+audio(fragmented=True)+atom(b'mvex',atom(b'trex',word(0,2,1,20000,0,0x02000000))))
    def fragment(i):
        h=atom(b'tfhd',word(0x02003a,2,ids[i],durations[i],len(packets[i]),0x02000000))
        def build(offset):return atom(b'moof',atom(b'mfhd',word(0,i+1))+atom(b'traf',h+atom(b'tfdt',word(0,sum(durations[:i])))+atom(b'trun',word(1,1,offset))))
        m=build(0);return build(len(m)+8)+atom(b'mdat',packets[i])
    files['pcm-fragmented.mp4']=init+b''.join(fragment(i)for i in range(6))
    # Same packets, but the first used setup is table entry 3 rather than 1.
    def reversed_setups(data,is_audio=False):
        out=b''
        for k,b in atoms(data):
            if k==b'trak':b=reversed_setups(b,b'soun' in b)
            elif k in [b'moov',b'mdia',b'minf',b'stbl',b'mvex',b'moof',b'traf']:b=reversed_setups(b,is_audio)
            elif is_audio and k==b'stsd':b=word(0,3)+b''.join(atom(t,v)for t,v in reversed(list(atoms(b[8:]))))
            elif is_audio and k==b'stsc':
                b=bytearray(b)
                for i in range(struct.unpack_from('>I',b,4)[0]):
                    at=16+12*i;b[at:at+4]=word(4-struct.unpack_from('>I',b,at)[0])
            elif k==b'trex' and struct.unpack_from('>I',b,4)[0]==2:b=bytearray(b);b[8:12]=word(4-struct.unpack_from('>I',b,8)[0])
            elif k==b'tfhd' and struct.unpack_from('>I',b,4)[0]==2:b=bytearray(b);b[8:12]=word(4-struct.unpack_from('>I',b,8)[0])
            out+=atom(k,b)
        return out
    files['pcm-start-third.mp4']=reversed_setups(files['pcm.mp4'])
    files['pcm-start-third-fragmented.mp4']=reversed_setups(files['pcm-fragmented.mp4'])
    if pcm_only:return files
    from generate_eac3_dependent import packet
    from generate_eac3_mp4_programmes import config
    from generate_eac3_dialogue_normalization import level
    encoded=[]
    for layout,rate_code in [(2,0),(1,2)]:
        encoded.append([level(packet(layout,False,0,0,'mixed',None,False,blocks_override=6,rate_code=rate_code,divisor=1<<i,gains=[64]+[None]*5,compression=32+i*8,compact=True)[0],25,23)for i in range(3)])
    def eac3_movie(groups,selection):
        video=dict(atoms(mp4([frame(8,'420',16,16)]*17,8,'420',(16,16),list(range(1,18)))))
        header=atom(b'ftyp',video[b'ftyp']);start=len(header)+8+len(video[b'mdat']);offsets=[];raws=[];timing=[];descriptions=[]
        for identity in selection:
            for raw in groups[identity-1]:offsets.append(start);start+=len(raw);raws.append(raw);timing.append(32000 if identity==1 else 48000);descriptions.append(identity)
        setups=[]
        for identity,(layout,rc,rate,channels)in enumerate([(2,0,48000,2),(1,2,32000,1)],1):
            e=bytearray(28);e[6:8]=struct.pack('>H',1);e[16:20]=struct.pack('>HH',channels,16);e[24:28]=word(rate<<16)
            setups.append(atom(b'ec-3',e+atom(b'dec3',config([(layout,False,rc,0)],groups[identity-1],rate))))
        n=len(raws);stbl=atom(b'stbl',atom(b'stsd',word(0,2)+b''.join(setups))+atom(b'stts',word(0,n)+b''.join(word(1,d)for d in timing))+atom(b'stsz',word(0,0,n,*map(len,raws)))+atom(b'stsc',word(0,n)+b''.join(word(i+1,1,d)for i,d in enumerate(descriptions)))+atom(b'stco',word(0,n,*offsets)))
        t=bytearray(84);t[:4]=word(3);t[12:16]=word(2);t[20:24]=word(17);t[40:44]=t[56:60]=word(65536);t[72:76]=word(0x40000000)
        mdhd=bytearray(24);mdhd[12:20]=word(1000000,sum(timing))
        trak=atom(b'trak',atom(b'tkhd',t)+atom(b'mdia',atom(b'mdhd',mdhd)+atom(b'hdlr',word(0,0)+b'soun'+bytes(12))+atom(b'minf',atom(b'dinf',atom(b'dref',word(0,1)+atom(b'url ',word(1))))+stbl)))
        return header+atom(b'mdat',video[b'mdat']+b''.join(raws))+atom(b'moov',video[b'moov']+trak)
    files['eac3.mp4']=eac3_movie(encoded,[1,2,1])
    # These each use one actual setup; the other entry is unselected and valid.
    files['eac3-control-1.mp4']=eac3_movie(encoded,[1])
    files['eac3-control-2.mp4']=eac3_movie(encoded,[2])
    files['manifest.json']=(json.dumps(dict(frames=records,provenance='Owned stereo16/48k,mono float32/32k,stereo24/44.1k PCM,exact float32 oracles,six20ms spans,owned VP9,E-AC-3 delayed tails with different rates/layouts and nontrivial line/RF/dialogue gains; no external codec,private media,FFmpeg or network.'),indent=2)+'\n').encode()
    return files
if __name__=='__main__':
    OUT.mkdir(parents=True,exist_ok=True)
    for name,data in generate().items():(OUT/name).write_bytes(data)
