"""Owned colored VP9 with explicit no-sample fragment intervals."""
from pathlib import Path
import struct,json,hashlib
from generate_mp4_seek_io_regressions import generate
from generate_vp9_chroma import atom,word
from generate_mp4_tfhd_defaults_regressions import atoms
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/mp4-empty-fragment'
def main():
    own=generate();parts=dict(atoms(own['three-colors.mp4']));packets=[own[f'frame-{i}.packet'] for i in range(3)]
    empty={b'stts':word(0,0),b'stsz':word(0,0,0),b'stsc':word(0,0),b'stco':word(0,0),b'stss':word(0,0)}
    def init(data):
        out=b''
        for k,b in atoms(data):
            if k in [b'trak',b'mdia',b'minf',b'stbl']:b=init(b)
            elif k in empty:b=empty[k]
            elif k==b'mdhd':b=b[:16]+word(0)+b[20:]
            elif k==b'mvhd':b=b[:16]+word(0)+b[20:]
            elif k==b'tkhd':b=b[:20]+word(0)+b[24:]
            out+=atom(k,b)
        return out
    prefix=atom(b'ftyp',b'iso5'+word(0)+b'iso5vp09')+atom(b'moov',init(parts[b'moov'])+atom(b'mvex',atom(b'trex',word(0,1,1,2,len(packets[0]),0))))
    def frag(seq,color=None,stamp=None,gap=None,inherit=False,run=False,version=0):
        flags=0x20000|8|16 if color is not None else 0x20000|0x10000|(0 if inherit else 8)
        body=word(flags,1)
        if color is not None:body+=word(1,len(packets[color]))
        elif not inherit:body+=word(gap)
        tfhd=atom(b'tfhd',body)
        tfdt=b'' if stamp is None else atom(b'tfdt',word(0,stamp) if version==0 else word(1<<24)+struct.pack('>Q',stamp))
        def moof(offset):
            trun=atom(b'trun',word(1,1 if color is not None else 0,offset)) if color is not None or run else b''
            return atom(b'moof',atom(b'mfhd',word(0,seq))+atom(b'traf',tfhd+tfdt+trun))
        m=moof(0);return moof(len(m)+8)+(atom(b'mdat',packets[color]) if color is not None else b'')
    files={};cases=[]
    def add(n,specs,pts,duration):
        files[n]=prefix+b''.join(frag(i+1,**x) for i,x in enumerate(specs));cases.append(dict(file=n,pts=pts,duration=duration))
    normal=[dict(color=i,stamp=i) for i in range(3)]
    add('plain.mp4',normal,[0,1,2],3)
    for v in [0,1]:
        add(f'middle-v{v}-explicit.mp4',[dict(color=0,stamp=0),dict(gap=2,stamp=1,version=v),dict(color=1,stamp=3),dict(color=2,stamp=4)],[0,3,4],5)
        add(f'middle-v{v}-implicit.mp4',[dict(color=0,stamp=0),dict(gap=2,stamp=1,version=v),dict(color=1),dict(color=2)],[0,3,4],5)
    add('leading.mp4',[dict(gap=2,stamp=0),dict(color=0),dict(color=1),dict(color=2)],[2,3,4],5)
    add('trailing.mp4',normal+[dict(gap=2,stamp=3)],[0,1,2],5)
    add('inherited.mp4',[dict(color=0,stamp=0),dict(gap=2,inherit=True),dict(color=1),dict(color=2)],[0,3,4],5)
    add('consecutive.mp4',[dict(color=0,stamp=0),dict(gap=1),dict(gap=1),dict(color=1),dict(color=2)],[0,3,4],5)
    add('zero-gap.mp4',[dict(color=0,stamp=0),dict(gap=0),dict(color=1),dict(color=2)],[0,1,2],3)
    files['contradictory-run.mp4']=prefix+frag(1,gap=2,stamp=0,run=True)+b''.join(frag(i+2,color=i,stamp=i) for i in range(3))
    files['overflow.mp4']=prefix+frag(1,gap=2,stamp=(1<<64)-1,version=1)+b''.join(frag(i+2,color=i,stamp=i) for i in range(3))
    for i in range(3):files[f'frame-{i}.yuv16']=own[f'frame-{i}.yuv16']
    OUT.mkdir(parents=True,exist_ok=True)
    for n,b in files.items():(OUT/n).write_bytes(b)
    (OUT/'manifest.json').write_text(json.dumps({'provenance':'Own 16x16 colored VP9, 50Hz, explicit empty-duration tfhd intervals without trun/mdat. Three pictures per valid file. No private media/parameters, FFmpeg or network.','cases':cases,'sha256':{n:hashlib.sha256(b).hexdigest() for n,b in files.items()}},indent=2)+'\n')
if __name__=='__main__':main()
