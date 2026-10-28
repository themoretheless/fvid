#!/usr/bin/env python3
"""Author owned VP9 timing fixtures; no external encoders or network."""
import hashlib,json
from pathlib import Path
from generate_vp9_chroma import frame,mp4,atom,word
from generate_mp4_tfhd_defaults_regressions import atoms
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/mp4-zero-default-duration'
def generate():
    packet=frame(8,'420',16,16)
    base=dict(atoms(mp4([packet]*3,8,'420',(16,16),[1,2,3])))
    empty={b'stts':word(0,0),b'stsz':word(0,0,0),b'stsc':word(0,0),b'stco':word(0,0),b'stss':word(0,0)}
    def rewrite(data):
        out=b''
        for k,b in atoms(data):
            if k in [b'trak',b'mdia',b'minf',b'stbl']:b=rewrite(b)
            elif k in [b'mdhd',b'mvhd']:b=b[:16]+word(0)+b[20:]
            else:b=empty.get(k,b)
            out+=atom(k,b)
        return out
    init=atom(b'ftyp',b'iso5'+word(0)+b'iso5vp09')+atom(b'moov',rewrite(base[b'moov'])+atom(b'mvex',atom(b'trex',word(0,1,1,1,len(packet),0))))
    files={'frame.vp9':packet}
    for name,duration in [('zero',0),('one',1),('two',2),('inherited',None)]:
        header=atom(b'tfhd',word(0x020010|(8 if duration is not None else 0),1)+(word(duration) if duration is not None else b'')+word(len(packet)))
        def moof(offset):
            first=atom(b'trun',word(1,1,offset))
            rest=atom(b'trun',word(0x100,2,1,1))
            return atom(b'moof',atom(b'mfhd',word(0,1))+atom(b'traf',header+atom(b'tfdt',word(0,0))+first+rest))
        files[name+'.mp4']=init+moof(len(moof(0))+8)+atom(b'mdat',packet*3)
    OUT.mkdir(parents=True,exist_ok=True)
    for n,b in files.items():(OUT/n).write_bytes(b)
    (OUT/'manifest.json').write_text(json.dumps({n:hashlib.sha256(b).hexdigest() for n,b in files.items()},indent=2)+'\n')
if __name__=='__main__':generate()
