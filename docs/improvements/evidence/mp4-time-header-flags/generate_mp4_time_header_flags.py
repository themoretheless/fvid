#!/usr/bin/env python3
"""Author owned VP9 media/mov header FullBox flag contract witnesses."""
from pathlib import Path
import json,hashlib,struct
from generate_vp9_chroma import frame,mp4,atom,word
from generate_mp4_tfhd_defaults_regressions import atoms
ROOT=Path(__file__).resolve().parents[1];OUT=ROOT/'tests/fixtures/playback-errors/mp4-time-header-flags'
def generate():
    packet=frame(8,'420',16,16);base=mp4([packet]*3,8,'420',(16,16),[1,2,3])
    def build(data,v,selected=None,flag=0):
        result=b''
        for k,b in atoms(data):
            if k in [b'moov',b'trak',b'mdia']:b=build(b,v,selected,flag)
            elif k in [b'mvhd',b'mdhd']:
                if v==1:b=word(0x01000000)+struct.pack('>QQIQ',int.from_bytes(b[4:8],'big'),int.from_bytes(b[8:12],'big'),int.from_bytes(b[12:16],'big'),int.from_bytes(b[16:20],'big'))+b[20:]
                if k==selected:b=b[:1]+flag.to_bytes(3,'big')+b[4:]
            result+=atom(k,b)
        return result
    files={'frame.vp9':packet};cases=[]
    for v in [0,1]:
        files[f'valid-v{v}.mp4']=build(base,v)
        for kind in [b'mvhd',b'mdhd']:
            for flag in [1,0x800000]:
                name=f'{kind.decode()}-v{v}-flags-{flag:06x}.mp4';files[name]=build(base,v,kind,flag);cases.append(name)
    OUT.mkdir(parents=True,exist_ok=True)
    for n,b in files.items():(OUT/n).write_bytes(b)
    (OUT/'manifest.json').write_text(json.dumps({'invalid_flags':cases,'sha256':{n:hashlib.sha256(b).hexdigest() for n,b in files.items()}},indent=2)+'\n')
if __name__=='__main__':generate()
