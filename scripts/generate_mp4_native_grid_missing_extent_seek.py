"""Owned finite PCM edit asking for unavailable source40..41ms."""
from pathlib import Path
import json,hashlib
from generate_mp4_native_grid_silence_seek import generate as controls
from generate_mp4_tfhd_defaults_regressions import atoms
from generate_vp9_chroma import atom,word
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/mp4-native-grid-missing-extent-seek'
def generate():
    source=controls()['changing-pcm-native-grid.mp4']
    def rewrite(data,audio=False):
        out=[]
        for k,b in atoms(data):
            if k==b'trak':b=rewrite(b,b'soun'in b)
            elif k in [b'moov',b'mdia',b'minf',b'stbl',b'edts']:b=rewrite(b,audio)
            elif audio and k==b'elst':b=word(0,2,1,0,65536,1,1920,65536)
            out.append(atom(k,b))
        return b''.join(out)
    name='past-source-edit.mp4';data=rewrite(source);return{name:data,'manifest.json':(json.dumps(dict(provenance='Own neutral VP9 and complete8k/16k PCM packets. Second finite edit requests40..41ms after the33.9375ms source extent. No private inputs,encoder or network.',sha256={name:hashlib.sha256(data).hexdigest()}),indent=2)+'\n').encode()}
if __name__=='__main__':
    OUT.mkdir(parents=True,exist_ok=True)
    for n,b in generate().items():(OUT/n).write_bytes(b)
