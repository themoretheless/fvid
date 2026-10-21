#!/usr/bin/env python3
"""Purely author VP9 fMP4 with effective fragment-description selection probes."""
import hashlib,json,struct
from pathlib import Path
from generate_vp9_chroma import frame,mp4,atom,word
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/mp4-fragment-description'
def atoms(data):
    at=0
    while at<len(data):
        size,kind=struct.unpack_from('>I4s',data,at);assert size>=8 and at+size<=len(data);yield kind,data[at+8:at+size];at+=size
    assert at==len(data)
def generate():
    packet=frame(8,'420',16,16);regular=mp4([packet]*3,8,'420',(16,16),[1,2,3]);original=dict(atoms(regular));files={'regular.mp4':regular,'frame.vp9':packet};controls=[];cases=[]
    empty={b'stts':word(0,0),b'stsz':word(0,0,0),b'stsc':word(0,0),b'stco':word(0,0),b'stss':word(0,0)}
    def rewrite(data):
        out=b''
        for k,b in atoms(data):
            if k in [b'trak',b'mdia',b'minf',b'stbl']:b=rewrite(b)
            elif k in empty:b=empty[k]
            out+=atom(k,b)
        return out
    init=rewrite(original[b'moov'])
    def fragment(sequence,dts,description,empty_run=False):
        flags=0x020038|(2 if description is not None else 0)
        tfhd=atom(b'tfhd',word(flags,1)+(word(description) if description is not None else b'')+word(1,len(packet),0x02000000))
        prefix=atom(b'mfhd',word(0,sequence));stamp=atom(b'tfdt',word(0,dts))
        def build(offset):return atom(b'moof',prefix+atom(b'traf',tfhd+stamp+atom(b'trun',word(1,0 if empty_run else 1,offset))))
        m=build(0);return build(len(m)+8)+atom(b'mdat',b'' if empty_run else packet)
    def build(default,descriptions,empty_zero=False):
        # iso5 is the profile that defines default-base-is-moof;avoid earlier brands.
        ftyp=atom(b'ftyp',b'iso5'+word(0)+b'iso5vp09')
        moov=atom(b'moov',init+atom(b'mvex',atom(b'trex',word(0,1,default,1,len(packet),0x02000000))))
        before=fragment(1,0,0,True) if empty_zero else b''
        return ftyp+moov+before+b''.join(fragment(i+1+int(empty_zero),i,d)for i,d in enumerate(descriptions))
    for name,default,descriptions,empty_zero in [('inherited-one',1,[None]*3,False),('explicit-one',1,[1]*3,False),('overridden-zero-default',0,[1]*3,False),('empty-zero-description',1,[None]*3,True)]:
        n=name+'.mp4';files[n]=build(default,descriptions,empty_zero);controls.append(dict(file=n,default=default,descriptions=descriptions,empty_zero=empty_zero))
    for name,default,descriptions in [('inherited-zero',0,[None]*3),('explicit-zero',1,[0]*3),('middle-zero',1,[1,0,1]),('late-zero',1,[1,1,0])]:
        n=name+'.mp4';files[n]=build(default,descriptions);cases.append(dict(file=n,default=default,descriptions=descriptions))
    # Existing out-of-range refusal must stay independent of newly rejected zero.
    files['explicit-two.mp4']=build(1,[2]*3)
    files['manifest.json']=(json.dumps(dict(provenance='Pure Python authored16x16 neutral VP9 fMP4,3 independent20ms frames. Effective trex/tfhd description-zero probes versus explicit/inherited1,overridden unused0 default,and empty unused0 tfhd. iso5 brand,whole moof/traf/trun/mdat geometry,known all128 planes and independently authored regular MP4 control. No private codec parameters,encoder,FFmpeg or network. Only supported direct stsd indexing is qualified;future stsd sample-group mappings are not implemented.',cases=cases,controls=controls,sha256={n:hashlib.sha256(b).hexdigest()for n,b in files.items()}),indent=2)+'\n').encode();return files
if __name__=='__main__':
    OUT.mkdir(parents=True,exist_ok=True)
    for n,b in generate().items():(OUT/n).write_bytes(b)
