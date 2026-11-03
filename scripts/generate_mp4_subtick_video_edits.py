"""Authored short VP9 edits below a video tick; pure offline generation."""
from pathlib import Path
import json,hashlib,struct
from generate_mp4_seek_io_regressions import generate as source
from generate_mp4_tfhd_defaults_regressions import atoms
from generate_vp9_chroma import atom,word,mp4
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/mp4-subtick-video-edits'
def generate():
    own=source();base=list(atoms(own['three-colors.mp4']))
    assert [k for k,_ in base]==[b'ftyp',b'mdat',b'moov']
    files={'unedited.mp4':own['three-colors.mp4']}
    for i in range(3):files[f'frame-{i}.yuv16']=own[f'frame-{i}.yuv16']
    specs=[('control-20',[(20,0)],[(0,20,0)]),('one',[(1,0)],[(0,1,0)]),('nineteen',[(19,0)],[(0,19,0)]),('twenty-one',[(21,0)],[(0,20,0),(20,21,1)]),('two-tiny',[(1,0),(1,1)],[(0,1,0),(1,2,1)]),('nineteen-plus-one',[(19,0),(1,1)],[(0,19,0),(19,20,1)]),('repeat-mixed',[(1,1),(21,0)],[(0,1,1),(1,21,0),(21,22,1)]),('leading-delay',[(5,-1),(1,0)],[(0,1,0)])]
    cases=[]
    for name,edits,frames in specs:
        length=sum(d for d,_ in edits)
        def rewrite(data):
            out=b''
            for k,b in atoms(data):
                if k==b'mvhd':b=b[:12]+word(1000,length)+b[20:]
                elif k==b'tkhd':b=b[:20]+word(length)+b[24:]
                elif k==b'trak':
                    b=rewrite(b)+atom(b'edts',atom(b'elst',word(0,len(edits))+b''.join(word(d)+struct.pack('>i',start)+word(0x10000)for d,start in edits)))
                out+=atom(k,b)
            return out
        files[name+'.mp4']=b''.join(atom(k,rewrite(b)if k==b'moov'else b)for k,b in base)
        cases.append(dict(file=name+'.mp4',edits=edits,frames=[dict(start_ms=a,end_ms=b,source=i)for a,b,i in frames],duration_ms=frames[-1][1]))
    from fractions import Fraction as F
    for c in cases:
        c['duration_ns']=c['duration_ms']*1_000_000
        for f in c['frames']:
            f['start_ns']=f['start_ms']*1_000_000;f['end_ns']=f['end_ms']*1_000_000
    prime=1_000_000_007;t=F(1_000_001,prime)
    extra=[('prime-short',[(1_000_001,0)],[(F(0),t,0)]),('prime-split',[(21_000_001,0)],[(F(0),F(1,50),0),(F(1,50),F(21_000_001,prime),1)]),('prime-two',[(1_000_001,0),(1_000_001,1)],[(F(0),t,0),(t,2*t,1)]),('prime-leading',[(3_000_005,-1),(1_000_001,0)],[(F(0),t,0)])]
    for name,edits,frames in extra:
        length=sum(d for d,_ in edits)
        def wide(data):
            out=b''
            for k,b in atoms(data):
                if k==b'mvhd':b=b[:12]+word(prime,length)+b[20:]
                elif k==b'tkhd':b=b[:20]+word(length)+b[24:]
                elif k==b'trak':b=wide(b)+atom(b'edts',atom(b'elst',word(0,len(edits))+b''.join(word(d)+struct.pack('>i',start)+word(0x10000)for d,start in edits)))
                out+=atom(k,b)
            return out
        files[name+'.mp4']=b''.join(atom(k,wide(b)if k==b'moov'else b)for k,b in base)
        cases.append(dict(file=name+'.mp4',edits=edits,duration_ns=int(frames[-1][1]*1_000_000_000),frames=[dict(start_ns=int(a*1_000_000_000),end_ns=int(b*1_000_000_000),source=i)for a,b,i in frames],common_grid_exceeds_u32=True))
    length=1;edits=[(1,0)]
    files['below-nanosecond.mp4']=b''.join(atom(k,wide(b)if k==b'moov'else b)for k,b in base)
    for name,offsets,media,chosen in [('negative-cut',[-1,-1,-1],0,1),('reordered-cut',[0,1,-1],1,2)]:
        original=files['one.mp4']
        def composed(data):
            out=b''
            for k,b in atoms(data):
                if k in [b'moov',b'trak',b'mdia',b'minf',b'stbl',b'edts']:b=composed(b)
                if k==b'stbl':b+=atom(b'ctts',word(1<<24,3)+b''.join(word(1)+struct.pack('>i',v)for v in offsets))
                if k==b'elst':b=word(0,1,1)+struct.pack('>i',media)+word(0x10000)
                out+=atom(k,b)
            return out
        files[name+'.mp4']=composed(original)
        cases.append(dict(file=name+'.mp4',edits=[(1,media)],duration_ns=1_000_000,frames=[dict(start_ns=0,end_ns=1_000_000,source=chosen)],raw_pts=[-1,0,1]if name=='negative-cut'else[0,1,2],raw_order=[0,1,2]if name=='negative-cut'else[0,2,1]))
    origin=(1<<63)-8
    empty={b'stts':word(0,0),b'stsz':word(0,0,0),b'stsc':word(0,0),b'stco':word(0,0),b'stss':word(0,0)}
    def anchored(data):
        out=b''
        for k,b in atoms(data):
            if k in [b'trak',b'mdia',b'minf',b'stbl']:b=anchored(b)
            if k in empty:b=empty[k]
            elif k==b'mvhd':b=b[:12]+word(1000,1)+b[20:]
            elif k==b'mdhd':b=b[:16]+word(0)+b[20:]
            elif k==b'tkhd':b=b[:20]+word(1)+b[24:]
            elif k==b'trak':b+=atom(b'edts',atom(b'elst',word(1<<24,1)+struct.pack('>Qq',1,origin)+word(0x10000)))
            out+=atom(k,b)
        return out
    pieces=dict(base);prefix=atom(b'ftyp',pieces[b'ftyp'])+atom(b'moov',anchored(pieces[b'moov'])+atom(b'mvex',atom(b'trex',word(0,1,1,1,len(own['frame-0.packet']),0))))
    def fragment(i):
        def moof(offset):return atom(b'moof',atom(b'mfhd',word(0,i+1))+atom(b'traf',atom(b'tfhd',word(0x20000,1))+atom(b'tfdt',word(1<<24)+struct.pack('>Q',origin+i))+atom(b'trun',word(1,1,offset))))
        first=moof(0);return moof(len(first)+8)+atom(b'mdat',own[f'frame-{i}.packet'])
    files['wide-origin.mp4']=prefix+b''.join(fragment(i)for i in range(3))
    cases.append(dict(file='wide-origin.mp4',edits=[(1,origin)],duration_ns=1_000_000,frames=[dict(start_ns=0,end_ns=1_000_000,source=0)],raw_pts=[origin+i for i in range(3)],raw_order=[0,1,2]))
    for c in cases:
        c.setdefault('raw_pts',[0,1,2]);c.setdefault('raw_order',[0,1,2])
    long=mp4([own[f'frame-{i%3}.packet']for i in range(64)],8,'420',size=(16,16),sync_samples=list(range(1,65)))
    def long_edit(data):
        out=b''
        for k,b in atoms(data):
            if k in [b'moov',b'trak']:b=long_edit(b)
            if k==b'mvhd':b=b[:12]+word(1000,1261)+b[20:]
            elif k==b'tkhd':b=b[:20]+word(1261)+b[24:]
            elif k==b'trak':b+=atom(b'edts',atom(b'elst',word(0,1,1261,0,0x10000)))
            out+=atom(k,b)
        return out
    files['seek-cost.mp4']=long_edit(long)
    files['manifest.json']=(json.dumps({'provenance':'Authored three16x16 VP9 keyframes at50Hz; movie edits at1000Hz and1000000007Hz. Packet span60ms, edited presentation1..22ms; a64-picture1.28s control checks late-seek packet reads. One fMP4 control places three packets near i64::MAX without large files/allocations; a separate subnanosecond edit is a precision refusal. Leading blank is skipped by established NativeReader policy. Scalar YUV oracles. No private media,encoder,FFmpeg or network.','cases':cases,'sha256':{n:hashlib.sha256(b).hexdigest()for n,b in files.items()}},indent=2)+'\n').encode()
    return files
if __name__=='__main__':
    OUT.mkdir(parents=True,exist_ok=True)
    for n,b in generate().items():(OUT/n).write_bytes(b)
