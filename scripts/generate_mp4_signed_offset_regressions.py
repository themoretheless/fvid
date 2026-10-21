#!/usr/bin/env python3
"""Author signed trun offset bounds,including sparse valid MIN displacement."""
import hashlib,json,struct
from pathlib import Path
from generate_mp4_fragment_description_regressions import generate as original,atoms,atom,word
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/mp4-signed-offset'
def generate():
    source=original();parts=dict(atoms(source['inherited-one.mp4']));head=atom(b'ftyp',parts[b'ftyp'])+atom(b'moov',parts[b'moov']);packet=source['frame.vp9'];media=atom(b'mdat',packet*3)
    def moof(shift):
        tfhd=atom(b'tfhd',word(0x020038,1,1,len(packet),0x02000000))
        return atom(b'moof',atom(b'mfhd',word(0,1))+atom(b'traf',tfhd+atom(b'tfdt',word(0,0))+atom(b'trun',word(1,3,shift&0xffffffff))))
    positive=head+moof(len(moof(0))+8)+media
    negative=head+media+moof(-3*len(packet))
    files={'positive.mp4':positive,'negative.mp4':negative,'frame.vp9':packet}
    malformed=[]
    for name,shift in [('min-underflow',-(1<<31)),('adjacent-underflow',-(1<<31)+1)]:
        n=name+'.mp4';files[n]=head+moof(shift)+media;malformed.append(dict(file=n,offset=shift,panics_before=shift==-(1<<31)))
    virtual=[]
    for key,displacement in [('valid-min',1<<31),('valid-adjacent',(1<<31)-1)]:
        body_at=len(head)+8;footer_at=body_at+displacement;free_at=len(head)+len(media);free_size=footer_at-free_at;assert 8<=free_size<1<<32
        prefix=head+media+struct.pack('>I4s',free_size,b'free');footer=moof(-displacement)
        files[key+'-prefix.bin']=prefix;files[key+'-footer.bin']=footer
        virtual.append(dict(name=key,size=footer_at+len(footer),offset=-displacement,segments=[dict(file=key+'-prefix.bin',offset=0),dict(file=key+'-footer.bin',offset=footer_at)]))
    files['manifest.json']=(json.dumps(dict(provenance='Pure own VP9:3 independent16x16 neutral keyframes at50Hz. Positive and negative in-file trun controls;small MIN/adjacent underflow faults. Stored sparse prefix/footer plans describe valid -2147483648/-2147483647 displacements over a standard ignored free box,without a2GiB stored file. Test adapters only interpret stored media segments;encoded-media generation is separate. All defaults/description IDs1,mdat bounds and sample clocks authored consistently. No private media,external codec,FFmpeg or network.',controls=['positive.mp4','negative.mp4'],malformed=malformed,virtual=virtual,sha256={n:hashlib.sha256(b).hexdigest()for n,b in files.items()}),indent=2)+'\n').encode();return files
if __name__=='__main__':
    OUT.mkdir(parents=True,exist_ok=True)
    for n,b in generate().items():(OUT/n).write_bytes(b)
