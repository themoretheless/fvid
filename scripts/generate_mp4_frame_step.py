#!/usr/bin/env python3
"""Short authored CFR/VFR MP4 frame-step controls, no encoder or network."""
from pathlib import Path
import hashlib,json,struct
from generate_mp4_seek_io_regressions import generate as seed_files
from generate_vp9_chroma import mp4,atom,word
from generate_mp4_fragment_description_regressions import atoms
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/mp4-frame-step'
def generate():
    seed=seed_files();files={f'frame-{i}.yuv16':seed[f'frame-{i}.yuv16']for i in range(3)};cases=[]
    source=mp4([seed[f'frame-{i%3}.packet']for i in range(5)],8,'420',size=(16,16),sync_samples=[1,2,3,4,5])
    for name,scale,durations in [('cfr',1000,[20]*5),('vfr',1000,[20,80,10,40,30]),('fractional',30000,[1001]*5)]:
        def walk(body):
            out=b''
            for k,b in atoms(body):
                if k in [b'moov',b'trak',b'mdia',b'minf',b'stbl']:b=walk(b)
                elif k in [b'mvhd',b'mdhd']:b=b[:12]+word(scale,sum(durations))+b[20:]
                elif k==b'tkhd':b=b[:20]+word(sum(durations))+b[24:]
                elif k==b'stts':b=word(0,len(durations))+b''.join(word(1,d)for d in durations)
                out+=atom(k,b)
            return out
        n=name+'.mp4';files[n]=walk(source);pts=[];at=0
        for d in durations:pts.append(at);at+=d
        cases.append({'file':n,'scale':scale,'pts':pts,'durations':durations})
    samples=[(i*17%1024)-512 for i in range(1600)]
    raw=struct.pack('<1600h',*samples)
    files['sound.wav']=b'RIFF'+struct.pack('<I',36+len(raw))+b'WAVEfmt '+struct.pack('<IHHIIHH',16,1,1,8000,16000,2,16)+b'data'+struct.pack('<I',len(raw))+raw
    files['sound-tail.f32le']=b''.join(struct.pack('<f',v/32768)for v in samples[800:])
    files['manifest.json']=(json.dumps({'provenance':'Five authored independently decodable16x16 VP9 keyframes using three colored packets and exact YUV planes. Only public synthetic timing tables vary. No private source,encoder or network.','cases':cases,'sha256':{n:hashlib.sha256(b).hexdigest()for n,b in files.items()}},indent=2)+'\n').encode()
    return files
if __name__=='__main__':
    OUT.mkdir(parents=True,exist_ok=True)
    for n,b in generate().items():(OUT/n).write_bytes(b)
