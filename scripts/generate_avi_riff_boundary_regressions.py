#!/usr/bin/env python3
"""Keep AVI headers,records and indices inside the declared RIFF form."""
import hashlib,json,struct
from pathlib import Path
from generate_avi_audio_eof_regressions import generate as owned,chunk
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/avi-riff-boundary'
def generate():
    original=owned();files={'video.mkv':original['video.mkv']};bad=[];controls=[];outside=[]
    for codec,frames in [('ima',27),('ms',12)]:
        base=original[codec+'.avi'];files[codec+'.f32le']=original[codec+'.f32le']
        movi=base.index(b'movi')-8
        for size in [0,2,4,movi-8]:
            b=bytearray(base);struct.pack_into('<I',b,4,size)
            name=f'{codec}-short-{size}.avi';files[name]=bytes(b);bad.append(dict(file=name,declared=size,frames=frames,expected=codec+'.f32le',control=codec+'-complete.avi'))
        for suffix,size in [('complete',len(base)-8),('overdeclared',len(base)+1000),('sentinel',0xffffffff)]:
            b=bytearray(base);struct.pack_into('<I',b,4,size);name=codec+'-'+suffix+'.avi';files[name]=bytes(b);controls.append(dict(file=name,frames=frames,expected=codec+'.f32le',profile='complete' if suffix=='complete' else 'existing clamped truncated-root compatibility'))
        name=codec+'-outside-index.avi';files[name]=base+chunk(b'idx1',struct.pack('<4sIII',b'00wb',0,4,999));outside.append(dict(file=name,frames=frames,expected=codec+'.f32le',control=codec+'-complete.avi',riff_end=len(base)))
    files['manifest.json']=(json.dumps(dict(provenance='Eight authored RIFF bounds place mandatory AVI lists outside the declared form. Two complete first-RIFF inputs append an outside idx1 that must not participate in its index validation. Six controls preserve complete and existing clamped overdeclared/sentinel profiles. Owned VP9/IMA-or-MS ADPCM AVI,independent predictor PCM,and separate owned Matroska video companion. No private media,encoder or network;not OpenDML or global validity qualification of trailing bytes.',cases=bad,controls=controls,outside=outside,sha256={n:hashlib.sha256(b).hexdigest() for n,b in files.items()}),indent=2)+'\n').encode();return files
if __name__=='__main__':
    OUT.mkdir(parents=True,exist_ok=True)
    for n,b in generate().items():(OUT/n).write_bytes(b)
