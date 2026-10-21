#!/usr/bin/env python3
"""Author BOS identity faults with valid CRCs and uninterrupted counters."""
import hashlib,json,struct
from pathlib import Path
from generate_ogg_page_sequence_regressions import page
from generate_audio_reader_eof_regressions import generate as owned
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/ogg-stream-identity'
def generate():
    head=b'OpusHead'+bytes([1,1])+struct.pack('<HIhB',0,48000,0,0)
    tags=b'OpusTags'+bytes(8);audio=bytes.fromhex('f8fffe');serial=0x46564920;other=serial+1
    files={'video.mkv':owned()['video.mkv'],'mono.head':head,'20ms.packet':audio};cases=[];controls=[]
    def stream(first_flags=2,tag_flags=0,audio_flags=0,start=serial,tag_serial=None,audio_serial=None):
        t=start if tag_serial is None else tag_serial
        a=t if audio_serial is None else audio_serial
        return b''.join([page(head,0,first_flags,0,start),page(tags,0,tag_flags,1,t),page(audio,960,audio_flags,2,a),page(audio,1920,4,3,a)])
    for name,start in [('normal',serial),('zero-serial',0)]:
        n=name+'.ogg';files[n]=stream(start=start);controls.append(dict(file=n,serial=start,frames=1920))
    for name,kw,initial,final in [
        ('missing-bos-zero',dict(first_flags=0,start=0),0,0),
        ('repeated-tags-bos',dict(tag_flags=2),serial,serial),
        ('foreign-tags-bos',dict(tag_flags=2,tag_serial=other),serial,other),
        ('repeated-audio-bos',dict(audio_flags=2),serial,serial),
        ('foreign-audio-bos',dict(audio_flags=2,audio_serial=other),serial,other),
    ]:
        n=name+'.ogg';files[n]=stream(**kw);cases.append(dict(file=n,header_serial=initial,before_final_serial=final,frames=1920))
    files['manifest.json']=(json.dumps(dict(provenance='Authored mono Opus headers and DTX packets. Each page has valid CRC and consecutive counters;no continuation flags. Missing/repeated BOS and foreign BOS cannot be hidden by prior sequence or continuation fixes. Owned VP9/PCM companion is separate from audio container reproduction. No private inputs,encoder or network.',cases=cases,controls=controls,sha256={n:hashlib.sha256(b).hexdigest() for n,b in files.items()}),indent=2)+'\n').encode()
    return files
if __name__=='__main__':
    OUT.mkdir(parents=True,exist_ok=True)
    for n,b in generate().items():(OUT/n).write_bytes(b)
