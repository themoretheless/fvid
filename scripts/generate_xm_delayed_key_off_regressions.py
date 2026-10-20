#!/usr/bin/env python3
"""Authored delayed key-off controls split at an independent note97 row."""
import json, struct, hashlib
from pathlib import Path
from generate_xm_key_off_regressions import generate as immediate, cell
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/xm-delayed-key-off'
def generate():
    authored=immediate();base=authored['key-off.xm'];at=60+struct.unpack_from('<I',base,60)[0];body=base[at+9+struct.unpack_from('<H',base,at+7)[0]:];header=base[:at]
    def module(rows):
        data=b''.join(rows);return header+struct.pack('<IBHH',9,0,len(rows),len(data))+data+body
    files={'video.mkv':authored['video.mkv']};cases=[]
    for param in [1,2,3,5,6,8,31,32,33,63,255]:
        tick=param&31; active=0<tick<6;name=f'key-off-{param:02x}'
        actual=[cell(49,1,effect=0x14,parameter=param),cell(),cell(97)]
        if active:
            control=[cell(49,1,effect=0x0f,parameter=tick),cell(97,effect=0x0f,parameter=6-tick),cell(effect=0x0f,parameter=6),cell(97)]
        else:control=[cell(49,1),cell(),cell(97)]
        files[name+'.xm']=module(actual);files[name+'-control.xm']=module(control)
        cases.append(dict(name=name,file=name+'.xm',control=name+'-control.xm',tick=tick,active=active))
    files['manifest.json']=(json.dumps({'provenance':'Authored delayed Kxx versus independent note97 at split tick rows. FT2 command masks parameter to5bits on nonzero ticks; zero masked or beyond row has no action. Signed16loop,no envelopes/fadeouts,authored VP9/PCM companion. No external encoder/network/private media.','cases':cases,'sha256':{n:hashlib.sha256(b).hexdigest() for n,b in files.items()}},indent=2)+'\n').encode();return files
if __name__=='__main__':
    OUT.mkdir(parents=True,exist_ok=True)
    for n,b in generate().items():(OUT/n).write_bytes(b)
