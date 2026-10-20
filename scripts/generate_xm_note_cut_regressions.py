#!/usr/bin/env python3
"""Authored ECx versus independent volume-column zero at a split row."""
import json, struct, hashlib
from pathlib import Path
from generate_xm_key_off_regressions import generate as immediate, cell
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/xm-note-cut'
def generate():
    authored=immediate();base=authored['key-off.xm'];at=60+struct.unpack_from('<I',base,60)[0];body=base[at+9+struct.unpack_from('<H',base,at+7)[0]:];header=base[:at]
    def module(rows,channels=1):
        head=bytearray(header);struct.pack_into('<H',head,68,channels)
        data=b''.join(rows);return bytes(head)+struct.pack('<IBHH',9,0,len(rows),len(data))+data+body
    files={'video.mkv':authored['video.mkv']};cases=[]
    for tick in [0,1,2,3,5,6,8,15]:
        for restore in [False,True]:
            name=f'note-cut-{tick:x}'+('-restore' if restore else '')
            active=tick<6
            actual=[cell(49,1,effect=0x0e,parameter=0xc0|tick),cell(effect=0x0c,parameter=64) if restore else cell(),cell(97)]
            if 0<tick<6:
                control=[cell(49,1,effect=0x0f,parameter=tick),cell(volume=0x10,effect=0x0f,parameter=6-tick),cell(volume=0x50 if restore else 0,effect=0x0f,parameter=6),cell(97)]
            else:
                control=[cell(49,1,volume=0x10 if tick==0 else 0),cell(effect=0x0c,parameter=64) if restore else cell(),cell(97)]
            files[name+'.xm']=module(actual);files[name+'-control.xm']=module(control)
            cases.append(dict(name=name,file=name+'.xm',control=name+'-control.xm',tick=tick,active=active,restore=restore))
    name='mixed-cut-and-key-off'
    actual=[cell(49,1,effect=0x0e,parameter=0xc1)+cell(50,1,effect=0x14,parameter=3),cell(effect=0x0c,parameter=64)+cell(effect=0x0c,parameter=64),cell(97)+cell(97)]
    control=[cell(49,1,effect=0x0f,parameter=1)+cell(50,1),cell(volume=0x10,effect=0x0f,parameter=2)+cell(),cell(effect=0x0f,parameter=3)+cell(97),cell(volume=0x50,effect=0x0f,parameter=6)+cell(volume=0x50),cell(97)+cell(97)]
    files[name+'.xm']=module(actual,2);files[name+'-control.xm']=module(control,2)
    cases.append(dict(name=name,file=name+'.xm',control=name+'-control.xm',active=True,restore=True,mixed=True))
    files['manifest.json']=(json.dumps({'provenance':'Authored ECx inputs versus independent volume-column zero at split tick rows. Immediate EC0,delayed ticks,out-of-row no-op and restoration without retrigger. Signed16loop,no envelopes/fadeouts,authored VP9/PCM companion. No external encoder/network/private media.','cases':cases,'sha256':{n:hashlib.sha256(b).hexdigest() for n,b in files.items()}},indent=2)+'\n').encode();return files
if __name__=='__main__':
    OUT.mkdir(parents=True,exist_ok=True)
    for n,b in generate().items():(OUT/n).write_bytes(b)
