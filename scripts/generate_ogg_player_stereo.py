"""Encode invented per-speaker tones; ordinary tests use checked-in Ogg files."""
from pathlib import Path
import tempfile,subprocess,math,struct,json,hashlib,os,itertools
from generate_ogg_audio_selection_budget_regressions import ogg_crc,ogg_page
from generate_mp4_seek_io_regressions import generate as video
ENCODER=os.environ.get('FFMPEG','ffmpeg')
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/ogg-player-stereo'
WAVE={1:['FC'],2:['FL','FR'],3:['FL','FR','FC'],4:['FL','FR','BL','BR'],5:['FL','FR','FC','BL','BR'],6:['FL','FR','FC','LFE','BL','BR'],7:['FL','FR','FC','LFE','BC','SL','SR'],8:['FL','FR','FC','LFE','BL','BR','SL','SR']}
VORBIS={1:['FC'],2:['FL','FR'],3:['FL','FC','FR'],4:['FL','FR','BL','BR'],5:['FL','FC','FR','BL','BR'],6:['FL','FC','FR','BL','BR','LFE'],7:['FL','FC','FR','SL','SR','BC','LFE'],8:['FL','FC','FR','SL','SR','BL','BR','LFE']}
BITS={'FL':1,'FR':2,'FC':4,'LFE':8,'BL':16,'BR':32,'BC':256,'SL':512,'SR':1024}
LAYOUT={1:'mono',2:'stereo',3:'3.0',4:'quad',5:'5.0',6:'5.1',7:'6.1',8:'7.1'}
def pages(data):
    at=0
    while at<len(data):
        assert data[at:at+4]==b'OggS';segments=data[at+26];size=27+segments+sum(data[at+27:at+27+segments]);yield data[at:at+size];at+=size
    assert at==len(data)
def serial(data,number):
    result=[]
    for raw in pages(data):
        p=bytearray(raw);p[14:18]=struct.pack('<I',number);p[22:26]=bytes(4);p[22:26]=struct.pack('<I',ogg_crc(bytes(p)));result.append(bytes(p))
    return b''.join(result)
def generate():
    own=video();files={'neutral-video.mp4':own['three-colors.mp4'],**{f'frame-{i}.yuv16':own[f'frame-{i}.yuv16']for i in range(3)}};cases=[]
    version=subprocess.check_output([ENCODER,'-version'],text=True).splitlines()[0]
    with tempfile.TemporaryDirectory(prefix='fvid-ogg-player-stereo-')as directory:
        tmp=Path(directory)
        for kind in ['vorbis','opus']:
            for channels in range(1,9):
                roles=WAVE[channels];freqs=[80 if r=='LFE'else 200+100*i for i,r in enumerate(roles)];raw=b''.join(struct.pack('<f',.012*(i+1)*math.sin(2*math.pi*f*t/48000))for t in range(5760)for i,f in enumerate(freqs));source=tmp/'input.f32le'
                # Independent libopus roundtrip identifies these encoder input
                # permutations; retain the intended encoded-speaker tone oracle.
                permutation={5:[0,1,4,2,3],7:[0,1,4,3,5,2,6]}.get(channels)if kind=='opus'else None
                if permutation:raw=b''.join(row[i*4:i*4+4]for at in range(5760)for row in [raw[at*channels*4:(at+1)*channels*4]]for i in permutation)
                source.write_bytes(raw);dest=tmp/'encoded.ogg'
                cmd=[ENCODER,'-nostdin','-v','error','-y','-fflags','+bitexact','-f','f32le','-ar','48000','-ac',str(channels),'-channel_layout',LAYOUT[channels],'-i',str(source),'-map_metadata','-1','-flags:a','+bitexact','-c:a','libvorbis'if kind=='vorbis'else'libopus']
                cmd+=['-q:a','3']if kind=='vorbis'else['-b:a',str(64000*channels),'-frame_duration','20'];cmd+=['-f','ogg',str(dest)];subprocess.run(cmd,check=True)
                name=f'{kind}-{channels}.ogg';files[name]=serial(dest.read_bytes(),0x522000+(0 if kind=='vorbis'else 16)+channels);cases.append(dict(file=name,kind=kind,channels=channels,roles=VORBIS[channels]if kind=='vorbis'else roles,mask=sum(BITS[r]for r in roles),frequencies={r:f for r,f in zip(roles,freqs)}))
    for kind,links in [('vorbis',['vorbis-6.ogg','vorbis-7.ogg']),('opus',['opus-6.ogg','opus-7.ogg']),('mixed',['vorbis-6.ogg','opus-7.ogg']),('mixed',['opus-7.ogg','vorbis-6.ogg'])]:
        name='chain-'+'-'.join(n[:-4]for n in links)+'.ogg';files[name]=b''.join(files[n]for n in links);roles=['FL','FR','FC','LFE','BL','BR','BC','SL','SR'];cases.append(dict(file=name,kind=kind,channels=9,roles=roles,mask=sum(BITS[r]for r in roles),links=links))
    a=list(pages(files['vorbis-6.ogg']));b=list(pages(files['opus-3.ogg']))
    files['multiplex.ogg']=a[0]+b[0]+a[1]+b[1]+b''.join(p for pair in itertools.zip_longest(a[2:],b[2:])for p in pair if p is not None)
    for index,base in enumerate(['vorbis-6.ogg','opus-3.ogg']):
        case=next(c for c in cases if c['file']==base).copy();case.update(file='multiplex.ogg',index=index);cases.append(case)
    original=list(pages(files['opus-6.ogg']));first=original[0];count=first[26];head=first[27+count:];assert head.startswith(b'OpusHead')and head[9]==6 and head[18]==1
    mapping=head[21:27]+head[21:23]+b'\xff';new=head[:9]+bytes([9])+head[10:18]+bytes([255])+head[19:21]+mapping;name='opus-discrete-9.ogg';number=0x522100;files[name]=ogg_page(number,0,0,2,[new])+serial(b''.join(original[1:]),number);cases.append(dict(file=name,kind='opus',channels=9,roles=[f'D{i}'for i in range(9)],mask=0,discrete=True))
    files['manifest.json']=(json.dumps(dict(provenance='Invented5760-frame48kHz per-channel sine float32;amplitude.012*(channel+1),LFE80Hz,other frequency200+100*WAVE-index. Encoded with FFmpeg/libvorbis and libopus as separate generation only. Explicit five/seven-channel Opus encoder input permutation verified by independent libopus reference;expected encoded-speaker tones remain unchanged. Own deterministic serials/CRC;own linked streams and explicit discrete duplicate/silent Opus map. Neutral3-frame16x16 VP9. No private source media/parameters;ordinary tests require neither FFmpeg nor network.',encoder=version,vorbis_channel_order_reference='https://xiph.org/vorbis/doc/Vorbis_I_spec.html#x1-800004.3.9',cases=cases,sha256={n:hashlib.sha256(b).hexdigest()for n,b in files.items()}),indent=2)+'\n').encode();return files
if __name__=='__main__':
    OUT.mkdir(parents=True,exist_ok=True)
    for n,b in generate().items():(OUT/n).write_bytes(b)
