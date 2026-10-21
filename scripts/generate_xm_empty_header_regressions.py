#!/usr/bin/env python3
"""Author short sample-free instrument headers, alignment and refusal controls."""
import hashlib
import json
import struct
from pathlib import Path
from generate_xm_key_off_regressions import generate as owned,cell
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/xm-empty-header'

def generate():
    original=owned();base=original['key-off.xm'];pattern=60+struct.unpack_from('<I',base,60)[0]
    header=base[:pattern];body=base[pattern+9+struct.unpack_from('<H',base,pattern+7)[0]:]
    def empty(size,pad=0):
        assert size>=29
        b=bytearray([pad]*size);b[:29]=bytes(29);struct.pack_into('<I',b,0,size)
        name=b'authored empty';b[4:4+len(name)]=name
        return bytes(b)
    def module(parts,rows):
        h=bytearray(header);struct.pack_into('<H',h,72,len(parts));data=b''.join(rows)
        return bytes(h)+struct.pack('<IBHH',9,0,len(rows),len(data))+data+b''.join(parts)
    files={'video.mkv':original['video.mkv']};cases=[]
    for name,size,placement,pad in [
        ('last-29',29,'last',0),('first-29',29,'first',0),('middle-29',29,'middle',0),
        ('padded-64',64,'last',0xA5),('padded-240',240,'last',0xA5),
        ('empty-only-29',29,'only',0),('full-241-control',241,'last',0),('full-243-control',243,'last',0),
    ]:
        e=empty(size,pad);c=empty(243)
        if placement=='first':parts=[e,body];controls=[c,body];rows=[cell(49,2),cell(),cell(97)]
        elif placement=='middle':parts=[body,e,body];controls=[body,c,body];rows=[cell(49,1),cell(55,3),cell(97)]
        elif placement=='only':parts=[e];controls=[c];rows=[cell(),cell(),cell()]
        else:parts=[body,e];controls=[body,c];rows=[cell(49,1),cell(),cell(97)]
        files[name+'.xm']=module(parts,rows);files[name+'-control.xm']=module(controls,rows)
        cases.append(dict(name=name,file=name+'.xm',control=name+'-control.xm',before_failure=size<241,empty_only=placement=='only',instruments=len(parts),duration_ms=480,post_row=1))
    rows=[cell(49,1),cell(),cell(97)]
    short_nonempty=bytearray(empty(29));struct.pack_into('<H',short_nonempty,27,1)
    huge=bytearray(empty(29));struct.pack_into('<I',huge,0,0xFFFFFFFF)
    for name,tail in [('too-short-28',struct.pack('<I',28)+bytes(24)),('nonempty-29',bytes(short_nonempty)),('truncated-29',empty(29)[:-1]),('huge-header',bytes(huge))]:
        files['bad-'+name+'.xm']=module([body,tail],rows)
    files['manifest.json']=(json.dumps(dict(provenance='Eight author-created XM modules with unused empty instrument headers at first,middle,last or all-empty positions. Six short-header failures and two existing full-header controls. Independent references use243-byte zero-sample headers. Four malformed variants require refusal. Owned signed16 ramp and authored VP9/PCM companion only;no encoder,network or private media.',cases=cases,malformed=['bad-'+n+'.xm' for n in ['too-short-28','nonempty-29','truncated-29','huge-header']],sha256={n:hashlib.sha256(b).hexdigest() for n,b in files.items()}),indent=2)+'\n').encode();return files
if __name__=='__main__':
    OUT.mkdir(parents=True,exist_ok=True)
    for n,b in generate().items():(OUT/n).write_bytes(b)
