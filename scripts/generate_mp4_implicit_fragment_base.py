#!/usr/bin/env python3
"""Own deterministic VP9 fragments covering ISO BMFF implicit tfhd data bases."""
from pathlib import Path
import struct, json, hashlib
from generate_vp9_chroma import atom, word
from generate_mp4_tfhd_defaults_regressions import atoms
ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / 'tests/fixtures/playback-errors/mp4-implicit-fragment-base'


def generate():
    owned = ROOT / 'tests/fixtures/playback-errors/mp4-tfhd-defaults'
    packet = (owned / 'frame.vp9').read_bytes()
    original = dict(atoms((owned / 'tfhd-overrides-trex-nonsync.mp4').read_bytes()))
    movie = list(atoms(original[b'moov']))
    template = next(b for k, b in movie if k == b'trak')
    ftyp = atom(b'ftyp', original[b'ftyp'])
    def track(data, ident, enabled, foreign):
        out = b''
        for k, b in atoms(data):
            if k in [b'mdia', b'minf', b'stbl']:
                b = track(b, ident, enabled, foreign)
            elif k == b'tkhd':
                b = word(3 if enabled else 0) + b[4:12] + word(ident) + b[16:]
            elif k == b'stsd' and foreign:
                entries = list(atoms(b[8:]));assert len(entries) == 1
                b = b[:8] + atom(b'zzzz', entries[0][1])
            out += atom(k, b)
        return out
    files = {'frame.vp9': packet};valid=[];invalid=[]
    definitions = [
        ('moof-control', [('moof', False, False)]),
        ('single-implicit', [('implicit', False, False)]),
        ('two-implicit', [('implicit', False, False), ('implicit', False, False)]),
        ('explicit-before-implicit', [('explicit', False, False), ('implicit', False, False)]),
        ('moof-before-implicit', [('moof', False, False), ('implicit', False, False)]),
        ('implicit-before-moof', [('implicit', False, False), ('moof', False, False)]),
        ('foreign-default-before-implicit', [('moof', True, False), ('implicit', False, False)]),
        ('foreign-rows-before-implicit', [('moof', True, True), ('implicit', False, False)]),
        ('outside-mdat-implicit', [('implicit', False, False)]),
        ('unresolved-foreign-before-implicit', [('moof', True, True), ('implicit', False, False)]),
        ('huge-foreign-before-implicit', [('moof', True, False), ('implicit', False, False)]),
    ]
    for name, specs in definitions:
        unresolved = name.startswith('unresolved-')
        parts=[];tracks=[];extends=[];layout=[]
        for i, (base, foreign, rows) in enumerate(specs):
            ident=99 if foreign else i+1
            count=2 if foreign else 3
            sizes=[7,9] if rows else [8]*count if foreign else [len(packet)]*count
            data=b'0123456789abcdef' if foreign else packet*count
            assert len(data) == sum(sizes)
            layout.append((ident, base, foreign, rows, count, sizes, sum(map(len, parts))))
            parts.append(data)
            tracks.append(atom(b'trak', track(template, ident, i==len(specs)-1, foreign)))
            extends.append(atom(b'trex', word(0, ident, 1, 1, 0 if rows else sizes[0], 0)))
        moov=atom(b'moov', b''.join(atom(k,b) for k,b in movie if k not in [b'trak',b'mvex']) + b''.join(tracks) + atom(b'mvex',b''.join(extends)))
        prefix=ftyp+moov
        def moof(length):
            trafs=[]
            for i,(ident,base,foreign,rows,count,sizes,start) in enumerate(layout):
                flags=0x8 | (0 if rows else 0x10) | (0x20000 if base=='moof' else 1 if base=='explicit' else 0)
                body=word(flags,ident)
                if base=='explicit':body+=struct.pack('>Q',len(prefix)+length+8+start)
                body+=word(1)
                if not rows:body+=word(sizes[0])
                offset=(base!='explicit' and (i==0 or base=='moof'))
                trflags=(1 if offset else 0) | (0x200 if rows and not unresolved else 0)
                run=word(trflags,0xffffffff if name.startswith('huge-') and foreign else count)
                if offset:run+=word(0 if name=='outside-mdat-implicit' else length+8+start)
                if trflags & 0x200:run+=word(*sizes)
                trafs.append(atom(b'traf',atom(b'tfhd',body)+atom(b'tfdt',word(0,0))+atom(b'trun',run)))
            return atom(b'moof',atom(b'mfhd',word(0,1))+b''.join(trafs))
        length=len(moof(0));fragment=moof(length);assert len(fragment)==length
        n=name+'.mp4';files[n]=prefix+fragment+atom(b'mdat',b''.join(parts))
        if name in ['outside-mdat-implicit','unresolved-foreign-before-implicit','huge-foreign-before-implicit']:
            invalid.append({'file':n,'error':'sample chunk outside mdat' if name.startswith('outside-') else 'MP4 implicit fragment base follows unresolved track data'})
        else:
            valid.append({'file':n,'tracks':[{'id':ident,'offset':len(prefix)+length+8+start,'offsets':[len(prefix)+length+8+start+i*len(packet) for i in range(3)]} for ident,_,foreign,_,_,_,start in layout if not foreign]})
    # Every moof starts a new legacy base, even after an earlier implicit traf.
    base_file=files['single-implicit.mp4']
    top=list(atoms(base_file));body=next(b for k,b in top if k==b'moof')
    def second(data):
        result=b''
        for k,b in atoms(data):
            if k==b'traf':b=second(b)
            elif k==b'mfhd':b=word(0,2)
            elif k==b'tfdt':b=word(0,3)
            result+=atom(k,b)
        return result
    new_moof=atom(b'moof',second(body))
    n='two-moofs-implicit.mp4';files[n]=base_file+new_moof+atom(b'mdat',packet*3)
    first=next(c for c in valid if c['file']=='single-implicit.mp4')['tracks'][0]['offsets']
    new_start=len(base_file)+len(new_moof)+8
    valid.append({'file':n,'tracks':[{'id':1,'offset':first[0],'offsets':first+[new_start+i*len(packet) for i in range(3)]}]})
    OUT.mkdir(parents=True,exist_ok=True)
    for n,b in files.items():(OUT/n).write_bytes(b)
    (OUT/'manifest.json').write_text(json.dumps({'provenance':'Owned pure-Python 16x16 neutral VP9, three 20ms frames per represented track. Synthetic unrepresented track uses literal dummy bytes, never decoded. No FFmpeg/network/private input.','valid':valid,'invalid':invalid,'sha256':{n:hashlib.sha256(b).hexdigest() for n,b in files.items()}},indent=2)+'\n')

if __name__=='__main__':generate()
