#!/usr/bin/env python3
"""Authored short XM timelines ending via Bxx or natural completion."""
import json,struct,hashlib
from pathlib import Path
from generate_xm_key_off_regressions import generate as immediate,cell
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/xm-row-budget'
def generate():
    authored=immediate();base=authored['key-off.xm'];at=60+struct.unpack_from('<I',base,60)[0];body=base[at+9+struct.unpack_from('<H',base,at+7)[0]:];header=base[:at]
    def module(rows):
        data=b''.join(rows);return header+struct.pack('<IBHH',9,0,len(rows),len(data))+data+body
    files={'video.mkv':authored['video.mkv']};cases=[]
    for count in [1,2,3]:
        rows=[cell(49,1)]+[cell() for _ in range(count-1)]
        normal=rows.copy();last=rows[-1];rows[-1]=last[:3]+bytes([11,1])
        name=f'end-jump-{count}';files[name+'.xm']=module(rows);files[name+'-natural.xm']=module(normal)
        cases.append(dict(file=name+'.xm',control=name+'-natural.xm',rows=count,short_budget=count-1))
    files['one-pass-loop.xm']=module([cell(49,1,effect=11,parameter=0)])
    files['manifest.json']=(json.dumps({'provenance':'Authored one-to-three row XM scores ending through B01 beyond the single-entry order list,matched natural-end controls and B00one-pass loop. Signed16loop,authored VP9/PCM companion. No external encoder,network or private media.','cases':cases,'sha256':{n:hashlib.sha256(b).hexdigest() for n,b in files.items()}},indent=2)+'\n').encode();return files
if __name__=='__main__':
    OUT.mkdir(parents=True,exist_ok=True)
    for n,b in generate().items():(OUT/n).write_bytes(b)
