#!/usr/bin/env python3
"""Authored Dxx targets beyond a short destination pattern."""
import json,struct,hashlib
from pathlib import Path
from generate_xm_key_off_regressions import generate as immediate,cell
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/xm-short-break'
def generate():
    authored=immediate();base=authored['key-off.xm'];at=60+struct.unpack_from('<I',base,60)[0];body=base[at+9+struct.unpack_from('<H',base,at+7)[0]:];header=base[:at];blank=cell()
    def module(patterns):
        head=bytearray(header);struct.pack_into('<H',head,64,len(patterns));struct.pack_into('<H',head,68,2);struct.pack_into('<H',head,70,len(patterns));head[80:336]=bytes(range(len(patterns)))+bytes(256-len(patterns));encoded=[]
        for rows in patterns:
            data=b''.join(rows);encoded.append(struct.pack('<IBHH',9,0,len(rows),len(data))+data)
        return bytes(head)+b''.join(encoded)+body
    destination=[cell(60,1)+blank,cell(97)+blank]
    files={'video.mkv':authored['video.mkv']};cases=[]
    for param in [0,1,2,3,0x63,0x64]:
        for jump in [False,True]:
            name=f'short-break-{param:02x}'+('-jump' if jump else '')
            first=(cell(49,1,effect=11,parameter=2)+cell(effect=13,parameter=param)) if jump else cell(49,1,effect=13,parameter=param)+blank
            patterns=[[first]]+([[cell(51,1)+blank,cell(52,1)+blank,cell(97)+blank]] if jump else [])+[destination]
            resume=1 if param==1 else 0
            actual=module(patterns);control=module([[cell(49,1)+blank]+destination[resume:]])
            files[name+'.xm']=actual;files[name+'-control.xm']=control
            target=2 if jump else 1
            cases.append(dict(name=name,file=name+'.xm',control=name+'-control.xm',timeline=[[0,0]]+[[target,i] for i in range(resume,2)],before_failure=param in [2,3,0x63]))
    files['manifest.json']=(json.dumps({'provenance':'Authored short2rowdestination withD00/D01controls,D02/D03/D63out-of-pattern targets andD64global-bound control. Both natural-next and earlier B02destinations. Literal traces and independent flattened notes;VP9/PCM companion. No external encoder,network or private media.','cases':cases,'sha256':{n:hashlib.sha256(b).hexdigest() for n,b in files.items()}},indent=2)+'\n').encode();return files
if __name__=='__main__':
    OUT.mkdir(parents=True,exist_ok=True)
    for n,b in generate().items():(OUT/n).write_bytes(b)
