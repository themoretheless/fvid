#!/usr/bin/env python3
"""Author invalid instrument components against independently normalized rows."""
import hashlib
import json
import struct
from pathlib import Path
from generate_xm_key_off_regressions import generate as owned,cell
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/xm-invalid-defaults'

def generate():
    original=owned();base=original['key-off.xm'];pattern=60+struct.unpack_from('<I',base,60)[0]
    header=base[:pattern];body=base[pattern+9+struct.unpack_from('<H',base,pattern+7)[0]:]
    empty=bytearray(243);struct.pack_into('<I',empty,0,243)
    def module(rows,count=1):
        h=bytearray(header);struct.pack_into('<H',h,72,count);data=b''.join(rows)
        return bytes(h)+struct.pack('<IBHH',9,0,len(rows),len(data))+data+body+bytes(empty)*(count-1)
    files={'video.mkv':original['video.mkv']};cases=[]
    def add(name,rows,control,bad,kind,post=2,expected_missing=1,control_count=1):
        files[name+'.xm']=module(rows);files[name+'-control.xm']=module(control,control_count)
        cases.append(dict(name=name,file=name+'.xm',control=name+'-control.xm',before_failure=bad,kind=kind,post_row=post,duration_ms=720,expected_missing=expected_missing))
    for slot in [129,255]:
        for note in [55,0]:
            name=f'slot-{slot}-'+('note' if note else 'selection')+'-mute'
            rows=[cell(49,1),cell(effect=0x0C),cell(note,slot),cell(),cell(97)]
            control=rows.copy();control[2]=cell(note)
            add(name,rows,control,True,'unexpected-unmute')
        name=f'slot-{slot}-note-pan'
        rows=[cell(49,1),cell(effect=8,parameter=0),cell(55,slot),cell(),cell(97)]
        control=rows.copy();control[2]=cell(55)
        add(name,rows,control,True,'unexpected-pan-reset')
        name=f'slot-{slot}-key-off'
        rows=[cell(49,1),cell(instrument=slot,effect=0x14),cell(),cell(),cell(97)]
        control=rows.copy();control[1]=cell(effect=0x0C)
        add(name,rows,control,True,'unexpected-unmute',post=1)
    for name,pre,note in [('populated-note-defaults-control',cell(effect=0x0C),55),('populated-selection-defaults-control',cell(effect=8,parameter=0),0)]:
        rows=[cell(49,1),pre,cell(note,1),cell(),cell(97)]
        control=rows.copy();control[2]=cell(note,volume=0x50,effect=8,parameter=128)
        add(name,rows,control,False,'valid-default-restore',expected_missing=0)
    rows=[cell(49,1),cell(effect=0x0C),cell(instrument=128),cell(),cell(97)]
    add('valid-empty-slot-defaults-control',rows,rows,False,'valid-default-restore',control_count=128)
    rows=[cell(49,1),cell(instrument=128,effect=0x14),cell(),cell(),cell(97)]
    add('valid-empty-slot-key-off-control',rows,rows,False,'valid-default-restore',post=1,control_count=128)
    rows=[cell(49,1),cell(effect=0x0C),cell(55),cell(),cell(97)]
    add('zero-instrument-control',rows,rows,False,'silent-control',expected_missing=0)
    rows=[cell(49,1),cell(),cell(55,129),cell(),cell(97)]
    control=rows.copy();control[2]=cell(55)
    add('default-level-invalid-control',rows,control,False,'unchanged-default-level')
    files['manifest.json']=(json.dumps(dict(provenance='Fourteen authored five-row XM scores. Invalid instrument components129/255 must not reset channel defaults,while valid notes/effects still execute. Independent controls remove only the invalid component;K00 cases use independent C00 cuts. Valid populated/empty-slot/default-level controls preserve intended restores. Existing owned signed16 waveform and authored VP9/PCM companion only;no private input,encoder or network.',cases=cases,sha256={n:hashlib.sha256(b).hexdigest() for n,b in files.items()}),indent=2)+'\n').encode();return files
if __name__=='__main__':
    OUT.mkdir(parents=True,exist_ok=True)
    for n,b in generate().items():(OUT/n).write_bytes(b)
