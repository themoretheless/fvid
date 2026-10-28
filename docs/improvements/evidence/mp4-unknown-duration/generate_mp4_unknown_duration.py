#!/usr/bin/env python3
"""Owned short VP9 MP4 witnesses of mdhd's all-ones unknown duration."""
from pathlib import Path
import hashlib,json,struct
from generate_vp9_chroma import frame,mp4,atom,word
from generate_mp4_tfhd_defaults_regressions import atoms
ROOT=Path(__file__).resolve().parents[1];OUT=ROOT/'tests/fixtures/playback-errors/mp4-unknown-duration'
def generate():
    packet=frame(8,'420',16,16);regular=mp4([packet]*3,8,'420',(16,16),[1,2,3])
    fragmented=(ROOT/'tests/fixtures/playback-errors/mp4-zero-default-duration/one.mp4').read_bytes()
    def rewrite(data,v):
        result=b''
        for k,b in atoms(data):
            if k in [b'moov',b'trak',b'mdia']:b=rewrite(b,v)
            elif k==b'mdhd':
                assert b[0]==0
                if v==0:b=b[:16]+word(0xffffffff)+b[20:]
                else:b=word(0x01000000)+struct.pack('>QQIQ',0,0,int.from_bytes(b[12:16],'big'),0xffffffffffffffff)+b[20:]
            result+=atom(k,b)
        return result
    OUT.mkdir(parents=True,exist_ok=True);files={'frame.vp9':packet,'regular-known.mp4':regular,'fragmented-known.mp4':fragmented}
    for name,data in [('regular',regular),('fragmented',fragmented)]:
        for v in [0,1]:files[f'{name}-unknown-v{v}.mp4']=rewrite(data,v)
    for n,b in files.items():(OUT/n).write_bytes(b)
    (OUT/'manifest.json').write_text(json.dumps({n:hashlib.sha256(b).hexdigest() for n,b in files.items()},indent=2)+'\n')
if __name__=='__main__':generate()
