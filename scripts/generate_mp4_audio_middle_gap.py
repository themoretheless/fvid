"""Own VP9/AAC/PCM/ALAC with an explicit packet-free middle fragment."""
from pathlib import Path
import json,hashlib,struct,math
from generate_vp9_chroma import atom,word
from generate_mp4_tfhd_defaults_regressions import atoms
ROOT=Path(__file__).resolve().parents[1]
BASE=ROOT/'tests/fixtures/playback-errors/mp4-open-ended-edit'
OUT=ROOT/'tests/fixtures/playback-errors/mp4-audio-middle-gap'
def main():
    original=json.loads((BASE/'manifest-audio.json').read_text());files={};cases=[]
    synthetic=ROOT/'tests/fixtures/playback-errors'
    ps=next(c for c in json.loads((synthetic/'aac-ps-absence-oracles.json').read_text())['cases'] if c['name']=='late-ps' and c['slots']==16)
    psraw=(synthetic/ps['video']['file']).read_bytes()
    pstrack=next(b for k,b in atoms(dict(atoms(psraw))[b'moov']) if k==b'trak' and b'soun' in b)
    psstbl=dict(atoms(dict(atoms(dict(atoms(pstrack))[b'mdia']))[b'minf']))[b'stbl']
    psstsd=dict(atoms(psstbl))[b'stsd']
    blob=(synthetic/'he-aac-ps-absence-packets.bin').read_bytes()
    pspackets=[blob[f['offset']:f['offset']+f['bytes']] for f in ps['frames']]
    pcm=(synthetic/'aac-ps-absence-pcm.bin').read_bytes()
    psvalues=[struct.unpack_from('<'+str(n)+'d',pcm,off) for off,n in ps['pcm']['Double']]
    psoracle=b''.join(struct.pack('<ff',a,b) for a,b in zip(*psvalues))
    def ps_template(version,mode):
        raw=(BASE/(f'aac-v{version}-clock48000-trim0-'+('known.mp4' if mode=='finite' else 'open.mp4'))).read_bytes()
        step=2048*294
        def audio(data,a=False):
            out=b''
            for k,b in atoms(data):
                selected=a
                if k==b'trak':selected=b'soun' in b
                if k in [b'moov',b'trak',b'mdia',b'minf',b'stbl',b'edts']:b=audio(b,selected)
                elif k==b'stsd' and a:b=psstsd
                elif k==b'mdhd' and a:b=b[:16]+word(3*step)+b[20:]
                elif k==b'tkhd' and a and mode=='finite':b=b[:20]+word(6144)+b[24:]
                elif k==b'elst' and a and mode=='finite':b=word(version<<24,1)+(word(6144,0,65536) if version==0 else struct.pack('>QqI',6144,0,65536))
                out+=atom(k,b)
            return out
        def fragment(data,index):
            out=b''
            for k,b in atoms(data):
                if k==b'traf':b=fragment(b,index)
                elif k==b'tfhd':b=word(0x020018,2,step,len(pspackets[index]))
                elif k==b'tfdt':b=word(0,index*step)
                out+=atom(k,b)
            return out
        out=b'';i=0;pending=False
        for k,b in atoms(raw):
            if k==b'moov':b=audio(b)
            elif k==b'moof':b=fragment(b,i);pending=True
            elif k==b'mdat' and pending:b=pspackets[i];i+=1;pending=False
            out+=atom(k,b)
        assert i==3
        return out
    for codec,rate in [('aac',48000),('pcm',8000),('alac',8000),('ps',48000)]:
        if codec=='ps':
            files['ps-oracle.f32le']=psoracle
            c=dict(media_ticks=3*2048*294,frames=6144,channels=2)
        else:
            files[f'{codec}-oracle.f32le']=(BASE/f'{codec}-oracle.f32le').read_bytes()
            c=next(c for c in original['cases'] if c['file']==f'{codec}-v0-clock{rate}-trim0-open.mp4')
        step=c['media_ticks']//3;packet_frames=c['frames']//3
        for stamp in ['v0','v1','implicit']:
            version=int(stamp[-1]) if stamp!='implicit' else 0
            for mode in ['none','open','finite','repeat']:
                repeats=2 if mode=='repeat' else 1
                raw=ps_template(version,mode) if codec=='ps' else (BASE/(f'{codec}-v{version}-clock{rate}-trim0-'+('known.mp4' if mode=='finite' else 'open.mp4'))).read_bytes()
                parts=list(atoms(raw))
                for i,packet in enumerate([b for k,b in parts if k==b'mdat'][-3:]):files[f'{codec}-{i}.packet']=packet
                video=next(b for k,b in atoms(dict(parts)[b'moov']) if k==b'trak' and b'vide' in b)
                tables=dict(atoms(dict(atoms(dict(atoms(video))[b'mdia']))[b'minf']))[b'stbl'];video_frames=int.from_bytes(dict(atoms(tables))[b'stsz'][8:12],'big')
                def init(data,audio=False):
                    out=b''
                    for k,b in atoms(data):
                        selected=audio
                        if k==b'trak':selected=b'soun' in b
                        if k==b'edts' and audio and mode=='none':continue
                        if k in [b'moov',b'trak',b'edts']:b=init(b,selected)
                        elif k==b'elst' and audio and mode=='finite':b=word(version<<24,1)+(word(4*packet_frames,0,65536) if version==0 else struct.pack('>QqI',4*packet_frames,0,65536))
                        elif k==b'elst' and audio and mode=='repeat':
                            row=word(4*packet_frames,0,65536) if version==0 else struct.pack('>QqI',4*packet_frames,0,65536)
                            b=word(version<<24,2)+row+row
                        elif k==b'tkhd' and audio and mode!='open':b=b[:20]+word(4*packet_frames*repeats)+b[24:]
                        elif k==b'mvhd':b=b[:16]+word(max(int.from_bytes(b[16:20],'big'),4*packet_frames*repeats))+b[20:]
                        out+=atom(k,b)
                    return out
                def decode_time(value):return atom(b'tfdt',word(1<<24)+struct.pack('>Q',value) if stamp=='v1' else word(0,value))
                def fragment(data,index):
                    out=b''
                    for k,b in atoms(data):
                        if k==b'traf':b=fragment(b,index)
                        elif k==b'mfhd':b=word(0,1 if index==0 else index+2)
                        elif k==b'tfdt':
                            if stamp=='implicit' and index>0:continue
                            out+=decode_time([0,2*step,3*step][index]);continue
                        elif k==b'trun':
                            delta=4 if stamp=='v1' else (-16 if stamp=='implicit' and index>0 else 0)
                            b=b[:8]+word(int.from_bytes(b[8:12],'big')+delta)+b[12:]
                        out+=atom(k,b)
                    return out
                gap=atom(b'moof',atom(b'mfhd',word(0,2))+atom(b'traf',atom(b'tfhd',word(0x030008,2,step))+(b'' if stamp=='implicit' else decode_time(step))))
                out=b'';index=0;insert=False
                for k,b in parts:
                    if k==b'moov':b=init(b)
                    elif k==b'moof':b=fragment(b,index);index+=1;insert=index==1
                    out+=atom(k,b)
                    if k==b'mdat' and insert:out+=gap;insert=False
                assert index==3
                name=f'{codec}-{stamp}-{mode}.mp4';files[name]=out
                cases.append(dict(file=name,codec=codec,rate=rate,channels=c['channels'],packet_frames=packet_frames,frames=4*packet_frames*repeats,repeats=repeats,media_step=step,video_frames=video_frames,mode=mode,stamp=stamp))
    # Native 8 kHz packets on a deliberately quantized 1001 Hz container clock.
    def coarse(data,audio=False):
        out=b''
        for k,b in atoms(data):
            selected=audio
            if k==b'trak':selected=b'soun' in b
            if k in [b'moov',b'trak',b'mdia',b'moof',b'traf']:b=coarse(b,selected)
            elif k==b'mdhd' and audio:b=b[:12]+word(1001,60)+b[20:]
            elif k==b'tfhd':b=b[:8]+word(20)+b[12:]
            elif k==b'tfdt':b=word(0,int.from_bytes(b[4:],'big')//8)
            out+=atom(k,b)
        return out
    for codec in ['pcm','alac']:
        files[f'{codec}-coarse.mp4']=coarse(files[f'{codec}-v0-none.mp4'])
    # Three coded audio packets and pictures, but a two-day empty source span.
    # This is never rendered in full: regression checks seek and render 10 ms.
    long_gap_ticks=2*24*3600*8000
    long_end=long_gap_ticks+3*160
    def long_gap(data,audio=False):
        out=b''
        for k,b in atoms(data):
            selected=audio
            if k==b'trak':selected=b'soun' in b
            if k in [b'moov',b'trak',b'moof',b'traf']:b=long_gap(b,selected)
            elif k==b'mvhd':b=b[:16]+word(long_end)+b[20:]
            elif k==b'tkhd' and audio:b=b[:20]+word(long_end)+b[24:]
            elif k==b'tfhd' and int.from_bytes(b[:4],'big')&0x10000:b=b[:8]+word(long_gap_ticks)+b[12:]
            elif k==b'tfdt':
                stamp=int.from_bytes(b[4:],'big')
                if stamp>=320:b=word(0,stamp+long_gap_ticks-160)
            out+=atom(k,b)
        return out
    files['pcm-long-gap.mp4']=long_gap(files['pcm-v0-none.mp4'])
    refused=[]
    for codec in ['aac','alac']:
        parts=list(atoms(files[f'{codec}-v0-none.mp4']))
        index=[i for i,(k,b) in enumerate(parts) if k==b'mdat'][-3:][1]
        assert parts[index-1][0]==b'moof'
        def one_byte_sample(data):
            out=b''
            for k,b in atoms(data):
                if k==b'traf':b=one_byte_sample(b)
                elif k==b'tfhd':
                    assert len(b)==16 and int.from_bytes(b[4:8],'big')==2
                    b=b[:-4]+word(1)
                out+=atom(k,b)
            return out
        parts[index-1]=(b'moof',one_byte_sample(parts[index-1][1]));parts[index]=(b'mdat',b'\xff')
        name=f'{codec}-invalid-after-gap.mp4';files[name]=b''.join(atom(k,b) for k,b in parts);refused.append(name)
    # Independent scalar Blackman-windowed sinc reference, with edges
    # clamped. Keep both core run-normalization and whole-WAVE post-DSP references.
    for codec in ['aac','pcm','alac','ps']:
        c=next(c for c in cases if c['codec']==codec)
        channels=c['channels'];frames=c['packet_frames']
        samples=list(struct.iter_unpack('<'+'f'*channels,files[f'{codec}-oracle.f32le']))
        def resample(part,precise=False):
            out=bytearray()
            for k in range(len(part)*2):
                center=k//2;phase=(k%2)/2;weights=[]
                for delta in range(-32,33):
                    distance=delta-phase
                    if abs(distance)>=32:continue
                    w=.94*(math.sin(math.pi*.94*distance)/(math.pi*.94*distance) if abs(distance)>1e-12 else 1.)*(.42+.5*math.cos(math.pi*distance/32)+.08*math.cos(2*math.pi*distance/32))
                    weights.append((min(max(center+delta,0),len(part)-1),w))
                norm=sum(w for _,w in weights)
                out+=struct.pack('<'+('d' if precise else 'f')*channels,*[sum(part[j][c]*w for j,w in weights)/norm for c in range(channels)])
            return out
        files[f'{codec}-rational-2x.f32le']=resample(samples[:frames])+bytes(frames*2*channels*4)+resample(samples[frames:])
        files[f'{codec}-normalized-2x.f32le']=resample(samples[:frames]+[(0.,)*channels]*frames+samples[frames:])
        if codec=='pcm':files['pcm-rational-2x.f64le']=resample(samples[:frames],True)+bytes(frames*2*channels*8)+resample(samples[frames:],True)
    OUT.mkdir(parents=True,exist_ok=True)
    for n,b in files.items():(OUT/n).write_bytes(b)
    (OUT/'manifest.json').write_text(json.dumps(dict(provenance='Existing authored neutral VP9, pulse/analytic AAC, delayed PS with independent double-precision noise/QMF stereo oracle, and integer PCM/ALAC. Three encoded audio packets at media positions0,2,3 with an explicit one-packet-duration empty tfhd interval after the first. No edits, open edit, finite edit, or two repeated finite edits. Explicit V0/V1 and inherited tfdt. PCM oracle inserts literal zero frames between the first and second authored packet outputs; compressed history is retained. No private media/parameters, FFmpeg or network.',cases=cases,refused=refused,resource_control=dict(file='pcm-long-gap.mp4',gap_ticks=long_gap_ticks,end_ticks=long_end,scale=8000,encoded_packets=3,full_logical_pcm_bytes=long_end*4),sha256={n:hashlib.sha256(b).hexdigest() for n,b in files.items()}),indent=2)+'\n')
if __name__=='__main__':main()
