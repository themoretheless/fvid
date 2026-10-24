"""Owned MP4 sample-description routing fixtures, without external codecs or I/O."""
import struct
from pathlib import Path
from generate_vp9_chroma import frame, mp4, atom, word
from generate_mp4_fragment_description_regressions import atoms
OUT = Path(__file__).resolve().parents[1] / 'tests/fixtures/playback-errors/mp4-sample-descriptions'

def generate():
    files = {}
    for changing in [False, True]:
        label = 'changing' if changing else 'equivalent'
        packets = [frame(8, '420', 16, 16), frame(8, '420', 32 if changing else 16, 16), frame(8, '420', 16, 16)]
        for i,payload in enumerate(packets):
            files[f'{label}-frame-{i}.vp9']=payload
            files[f'{label}-control-{i}.mp4']=mp4([payload],8,'420',(32 if changing and i==1 else 16,16),[1])
        original = dict(atoms(mp4(packets, 8, '420', (16,16), [1,2,3])))
        offset = len(atom(b'ftyp', original[b'ftyp'])) + 8
        offsets = [offset]
        for p in packets[:-1]: offsets.append(offsets[-1]+len(p))
        def rewrite(data, empty=False, bad=None):
            out = b''
            for k,b in atoms(data):
                if k in [b'trak',b'mdia',b'minf',b'stbl']: b = rewrite(b,empty,bad)
                elif k == b'tkhd':
                    b=bytearray(b);b[76:84]=bytes(8)
                elif k == b'stsd':
                    entry = list(atoms(b[8:]))[0]
                    second = bytearray(entry[1]);second[24:26]=struct.pack('>H',32 if changing else 16)
                    b=word(0,2)+atom(*entry)+atom(entry[0],second)
                elif empty and k in [b'stts',b'stsc',b'stco',b'stss']: b=word(0,0)
                elif empty and k==b'stsz': b=word(0,0,0)
                elif k==b'stsc': b=word(0,3,1,1,1,2,1,2 if bad is None else bad,3,1,1)
                elif k==b'stco': b=word(0,3,*offsets)
                out+=atom(k,b)
            return out
        files[label+'.mp4']=atom(b'ftyp',original[b'ftyp'])+atom(b'mdat',original[b'mdat'])+atom(b'moov',rewrite(original[b'moov']))
        for bad in [0,3]:
            files[f'{label}-chunk-{bad}.mp4']=atom(b'ftyp',original[b'ftyp'])+atom(b'mdat',original[b'mdat'])+atom(b'moov',rewrite(original[b'moov'],bad=bad))
        def fragment(i,description):
            # Inherit description 2 for middle frame; explicitly select 1 otherwise.
            flags=0x020038 | (0 if description is None else 2)
            h=atom(b'tfhd',word(flags,1)+(b'' if description is None else word(description))+word(1,len(packets[i]),0x02000000))
            def build(at):
                return atom(b'moof',atom(b'mfhd',word(0,i+1))+atom(b'traf',h+atom(b'tfdt',word(0,i))+atom(b'trun',word(1,1,at))))
            m=build(0)
            return build(len(m)+8)+atom(b'mdat',packets[i])
        init=atom(b'ftyp',b'iso5'+word(0)+b'iso5vp09')+atom(b'moov',rewrite(original[b'moov'],empty=True)+atom(b'mvex',atom(b'trex',word(0,1,2,1,0,0x02000000))))
        files[label+'-fragmented.mp4']=init+b''.join(fragment(i,d)for i,d in enumerate([1,None,1]))
        files[label+'-fragment-3.mp4']=init+b''.join(fragment(i,d)for i,d in enumerate([1,3,1]))
        # The first description must not hide a malformed later sample entry.
        data=files[label+'.mp4'];at=data.index(b'stsd')+12;size=struct.unpack_from('>I',data,at)[0]
        bad=bytearray(data);bad[at+size+8+6:at+size+8+8]=b'\0\2'
        files[label+'-external-second.mp4']=bytes(bad)
    # Compact PCM indexing still needs the description belonging to each chunk.
    base=dict(atoms(mp4([frame(8,'420',16,16)]*3,8,'420',(16,16),[1,2,3])))
    start=len(atom(b'ftyp',base[b'ftyp']))+8
    def pcm(data):
        out=b''
        for k,b in atoms(data):
            if k in [b'trak',b'mdia',b'minf',b'stbl']: b=pcm(b)
            elif k==b'hdlr': b=word(0,0)+b'soun'+bytes(12)
            elif k==b'stsd':
                entries=[]
                for channels,depth in [(2,16),(1,32)]:
                    e=bytearray(28);e[6:8]=struct.pack('>H',1);e[16:20]=struct.pack('>HH',channels,depth);e[24:28]=word(48000<<16)
                    entries.append(atom(b'sowt',e))
                b=word(0,2)+b''.join(entries)
            elif k==b'stsz': b=word(0,4,6)
            elif k==b'stts': b=word(0,1,6,1)
            elif k==b'stsc': b=word(0,3,1,2,1,2,2,2,3,2,1)
            elif k==b'stco': b=word(0,3,start,start+8,start+16)
            elif k==b'stss': continue
            out+=atom(k,b)
        return out
    files['compact-pcm.mp4']=atom(b'ftyp',base[b'ftyp'])+atom(b'mdat',bytes(range(24)))+atom(b'moov',pcm(base[b'moov']))
    return files

if __name__=='__main__':
    OUT.mkdir(parents=True,exist_ok=True)
    for name,data in generate().items(): (OUT/name).write_bytes(data)
