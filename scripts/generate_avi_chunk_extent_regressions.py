#!/usr/bin/env python3
"""Author known AVI audio chunks whose declared bodies escape their parent run."""
import hashlib,json,struct
from pathlib import Path
from generate_avi_audio_eof_regressions import generate as adpcm,chunk,listing
from generate_avi_pcm_regressions import generate as pcm
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/avi-chunk-extent'
def generate():
    a=adpcm();p=pcm();files={'video.mkv':a['video.mkv']};controls=[];cases=[]
    sources=[('ima',a['ima.avi'],a['ima.f32le'],27,9),('ms',a['ms.avi'],a['ms.f32le'],12,4),('int16',p['int16-1ch.avi'],p['int16-1ch.f32le'],9,2),('float32',p['float32-2ch.avi'],p['float32-2ch.f32le'],9,2)]
    for codec,base,gold,frames,seek in sources:
        files[codec+'.f32le']=gold;at=base.index(b'movi')-8;assert at+8+struct.unpack_from('<I',base,at+4)[0]==len(base)
        for nested in [False,True]:
            key=codec+('-nested' if nested else '-direct');children=base[at+12:]
            data=bytearray(base[:at]+listing(b'movi',listing(b'rec ',children) if nested else children));struct.pack_into('<I',data,4,len(data)-8)
            name=key+'.avi';files[name]=bytes(data);controls.append(dict(file=name,expected=codec+'.f32le',frames=frames,seek=seek,nested=nested))
            record=data.rindex(b'01wb');size=struct.unpack_from('<I',data,record+4)[0];assert record+8+size==len(data)
            for suffix,claimed in [('long',size+8),('max',0xffffffff)]:
                bad=bytearray(data);struct.pack_into('<I',bad,record+4,claimed);name=key+'-'+suffix+'.avi';files[name]=bytes(bad);cases.append(dict(file=name,control=key+'.avi',expected=codec+'.f32le',frames=frames,seek=seek,record_offset=record,available=size,claimed=claimed,nested=nested))
            if not nested:
                oversized=bytearray(data);struct.pack_into('<I',oversized,4,0xffffffff);name=key+'-outer-sentinel.avi';files[name]=bytes(oversized);controls.append(dict(file=name,expected=codec+'.f32le',frames=frames,seek=seek,nested=False))
    files['manifest.json']=(json.dumps(dict(provenance='16 independently authored final audio-chunk extent contradictions:IMA/MS/int16/IEEEfloat32,direct/nested rec,short overclaim/u32MAX.12 complete controls include existing outer RIFF sentinel profile. Existing coded audio and owned VP9 are synthesized by offline generators;manual literal float32 gold and separate VP9 Matroska companion. Only chunk size changes in faults;whole root/list extents and valid available audio stay intact. No private media,FFmpeg or network.',cases=cases,controls=controls,sha256={n:hashlib.sha256(b).hexdigest()for n,b in files.items()}),indent=2)+'\n').encode();return files
if __name__=='__main__':
    OUT.mkdir(parents=True,exist_ok=True)
    for n,b in generate().items():(OUT/n).write_bytes(b)
