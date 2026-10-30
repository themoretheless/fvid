"""Authored VP9 with unsigned wide DTS and representable signed composition times."""
from pathlib import Path
import struct,json,hashlib
from generate_vp9_chroma import atom,word
from generate_mp4_tfhd_defaults_regressions import atoms
ROOT=Path(__file__).resolve().parents[1]
BASE=ROOT/'tests/fixtures/playback-errors/mp4-empty-fragment'
OUT=ROOT/'tests/fixtures/playback-errors/mp4-wide-decode-clock'
def main():
    parts=list(atoms((BASE/'plain.mp4').read_bytes()))
    def init(data):
        out=b''
        for k,b in atoms(data):
            if k in [b'trak',b'mdia']:b=init(b)
            elif k in [b'mvhd',b'mdhd']:b=b[:12]+word(1_000_000_000)+b[16:]
            out+=atom(k,b)
        return out
    moov=init(next(b for k,b in parts if k==b'moov'))
    prefix=atom(b'ftyp',next(b for k,b in parts if k==b'ftyp'))+atom(b'moov',moov)
    source=next(b for k,b in parts if k==b'moof')
    packet=next(b for k,b in parts if k==b'mdat')
    def frag(data,dts,shift):
        out=b''
        for k,b in atoms(data):
            if k==b'traf':b=frag(b,dts,shift)
            elif k==b'tfdt':b=word(1<<24)+struct.pack('>Q',dts)
            elif k==b'trun':b=word((1<<24)|0x801,1,int.from_bytes(b[8:12],'big')+8)+struct.pack('>i',shift)
            out+=atom(k,b)
        return out
    files={'frame.packet':packet,'frame.yuv16':(BASE/'frame-0.yuv16').read_bytes()};cases=[]
    for name,dts,shift in [('control',0,0),('signed-boundary',1<<63,-1),('negative-offset',1<<63,-(1<<31)),('true-overflow',1<<63,0)]:
        name+='.mp4';files[name]=prefix+atom(b'moof',frag(source,dts,shift))+atom(b'mdat',packet)
        cases.append(dict(file=name,dts=dts,shift=shift,pts=dts+shift,accepted=dts+shift<1<<63))
    OUT.mkdir(parents=True,exist_ok=True)
    for n,b in files.items():(OUT/n).write_bytes(b)
    (OUT/'manifest.json').write_text(json.dumps(dict(provenance='Existing authored colored 16x16 VP9 picture. tfdt V1 is unsigned; trun V1 composition offset is signed. No private media/parameters, network or FFmpeg.',cases=cases,sha256={n:hashlib.sha256(b).hexdigest() for n,b in files.items()}),indent=2)+'\n')
if __name__=='__main__':main()
