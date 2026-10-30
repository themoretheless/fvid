#!/usr/bin/env python3
"""Owned PCM/ALAC edit boundaries between sample instants (no resampling oracle dependency)."""
from pathlib import Path
import struct,json,hashlib
from generate_vp9_chroma import atom,word
from generate_mp4_tfhd_defaults_regressions import atoms
ROOT=Path(__file__).resolve().parents[1]
OUT=ROOT/'tests/fixtures/playback-errors/mp4-open-ended-edit'
def generate():
    files={};cases=[]
    for codec in ['pcm','alac']:
        source=(OUT/f'{codec}-v0-clock8000-trim0-open.mp4').read_bytes()
        for version in [0,1]:
            for opened in [False,True]:
                def rewrite(data,audio=False):
                    out=b''
                    for k,b in atoms(data):
                        if k in [b'moov',b'trak',b'mdia',b'edts',b'moof',b'traf']:
                            b=rewrite(b,b'soun' in b if k==b'trak' else audio)
                        elif k==b'mvhd':b=b[:12]+word(44100,2646)+b[20:]
                        elif k==b'tkhd':
                            duration=(0 if opened else 2645) if audio else 2646
                            b=b[:20]+word(duration)+b[24:]
                        elif k==b'mdhd' and audio:b=b[:12]+word(44100,2646)+b[20:]
                        elif k==b'elst' and audio:
                            duration=0 if opened else 2645
                            b=word(version<<24,1)+(word(duration,1,0x10000) if version==0 else struct.pack('>QqI',duration,1,0x10000))
                        elif k==b'tfhd':assert int.from_bytes(b[4:8],'big')==2;b=b[:8]+word(882)+b[12:]
                        elif k==b'tfdt':
                            value=int.from_bytes(b[4:8],'big');assert value*441%80==0
                            b=b[:4]+word(value*441//80)
                        out+=atom(k,b)
                    return out
                name=f'{codec}-v{version}-fractional-'+('open.mp4' if opened else 'known.mp4')
                files[name]=rewrite(source);cases.append(dict(file=name,codec=codec,version=version,open=opened,oracle=f'{codec}-oracle.f32le',duration_ticks=2645,scale=44100,rate=8000))
    for n,b in files.items():(OUT/n).write_bytes(b)
    (OUT/'manifest-fractional.json').write_text(json.dumps({'provenance':'Owned PCM/ALAC + VP9. 8000Hz PCM, 44100Hz media/movie ticks, media_start=1 tick. First retained sample is index1 at 102324ns presentation time. No private media, FFmpeg or network.','cases':cases,'sha256':{n:hashlib.sha256(b).hexdigest()for n,b in files.items()}},indent=2)+'\n')
if __name__=='__main__':generate()
