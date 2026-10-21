#!/usr/bin/env python3
"""Keep AVIMAINHEADER stream count inside its own declared chunk."""
import hashlib,json,struct
from pathlib import Path
from generate_avi_audio_eof_regressions import generate as owned,chunk
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/avi-main-header-bounds'
def generate():
    original=owned();files={'video.mkv':original['video.mkv']};bad=[];controls=[]
    for codec,frames in [('ima',27),('ms',12)]:
        base=original[codec+'.avi'];at=base.index(b'avih');size=struct.unpack_from('<I',base,at+4)[0];header=base[at+8:at+8+size]
        assert size==56 and struct.unpack_from('<I',header,24)[0]==2
        def replace(data):
            b=bytearray(base[:at]+data+base[at+8+size:]);struct.pack_into('<I',b,4,len(b)-8)
            struct.pack_into('<I',b,16,struct.unpack_from('<I',base,16)[0]+len(data)-8-size)
            return bytes(b)
        files[codec+'.f32le']=original[codec+'.f32le']
        for length in [0,16,20]:
            junk=bytearray(20 if length==0 else 4 if length==16 else 2)
            if length!=20:struct.pack_into('<I',junk,16 if length==0 else 0,2)
            data=chunk(b'avih',header[:length])+chunk(b'JUNK',bytes(junk))
            # The old out-of-chunk read borrows either JUNK bytes or JUNK's length.
            assert struct.unpack_from('<I',data,8+24)[0]==2
            name=f'{codec}-short-{length}.avi';files[name]=replace(data);bad.append(dict(file=name,declared_header=length,control=codec+'-standard.avi',expected=codec+'.f32le',frames=frames))
        for length in [56,64]:
            name=f'{codec}-'+('standard' if length==56 else 'padded')+'.avi'
            files[name]=replace(chunk(b'avih',header+bytes(length-56)))
            controls.append(dict(file=name,expected=codec+'.f32le',frames=frames))
    files['manifest.json']=(json.dumps(dict(provenance='Six malformed owned AVI VP9/ADPCM files declare an avih too short to carry dwStreams,with a later JUNK word/size equal to2. Four complete56/64-byte controls and literal predictor PCM. Same VP9 frame in owned Matroska companion. Refusal regression,not acceptance of malformed playback. Pure Python,no private media,encoder or network.',cases=bad,controls=controls,sha256={n:hashlib.sha256(b).hexdigest() for n,b in files.items()}),indent=2)+'\n').encode();return files
if __name__=='__main__':
    OUT.mkdir(parents=True,exist_ok=True)
    for n,b in generate().items():(OUT/n).write_bytes(b)
