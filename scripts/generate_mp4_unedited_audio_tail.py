"""Authored video/audio with an unedited, explicitly declared packet-free tail."""
from pathlib import Path
import json,struct,hashlib
from generate_vp9_chroma import atom,word
from generate_mp4_tfhd_defaults_regressions import atoms
ROOT=Path(__file__).resolve().parents[1]
BASE=ROOT/'tests/fixtures/playback-errors/mp4-audio-middle-gap'
OUT=ROOT/'tests/fixtures/playback-errors/mp4-unedited-audio-tail'
def main():
    original=json.loads((BASE/'manifest.json').read_text());files={};cases=[]
    for c in original['cases']:
        if c['mode']!='none' or c['stamp']!='v0':continue
        codec=c['codec'];step=c['media_step'];source=(BASE/c['file']).read_bytes()
        def move(data):
            out=b''
            for k,b in atoms(data):
                if k==b'traf':b=move(b)
                elif k==b'tfdt':
                    t=int.from_bytes(b[4:],'big');b=word(0,t-step if t>=2*step else t)
                out+=atom(k,b)
            return out
        out=b''
        for k,b in atoms(source):
            if k==b'moof':
                traf=dict(atoms(b))[b'traf'];tfhd=dict(atoms(traf))[b'tfhd']
                if int.from_bytes(tfhd[:4],'big')&0x10000:continue
                b=move(b)
            out+=atom(k,b)
        tail=atom(b'moof',atom(b'mfhd',word(0,5))+atom(b'traf',atom(b'tfhd',word(0x030008,2,step))+atom(b'tfdt',word(0,3*step))))
        name=f'{codec}.mp4';files[name]=out+tail;files[f'{codec}-oracle.f32le']=(BASE/f'{codec}-oracle.f32le').read_bytes()+bytes(c['packet_frames']*c['channels']*4)
        cases.append(dict(file=name,codec=codec,rate=c['rate'],channels=c['channels'],frames=c['packet_frames']*4,packet_frames=c['packet_frames'],step=step,video_frames=c['video_frames']))
    gap=2*24*3600*8000
    def long_tail(data,audio=False):
        out=b''
        for k,b in atoms(data):
            selected=audio
            if k==b'trak':selected=b'soun' in b
            if k in [b'moov',b'trak',b'moof',b'traf']:b=long_tail(b,selected)
            elif k==b'mvhd':b=b[:16]+word(480+gap)+b[20:]
            elif k==b'tkhd' and audio:b=b[:20]+word(480+gap)+b[24:]
            elif k==b'tfhd' and int.from_bytes(b[:4],'big')&0x10000:b=b[:8]+word(gap)+b[12:]
            out+=atom(k,b)
        return out
    files['pcm-two-day-tail.mp4']=long_tail(files['pcm.mp4'])
    OUT.mkdir(parents=True,exist_ok=True)
    for n,b in files.items():(OUT/n).write_bytes(b)
    (OUT/'manifest.json').write_text(json.dumps(dict(provenance='Existing authored VP9/AAC/PS/PCM/ALAC packets. Three contiguous audio packets followed by one duration-is-empty fragment, no edit list. No private media/parameters, FFmpeg or network.',cases=cases,sha256={n:hashlib.sha256(b).hexdigest() for n,b in files.items()}),indent=2)+'\n')
if __name__=='__main__':main()
