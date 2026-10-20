#!/usr/bin/env python3
"""Authored combined Bxx/Dxx navigation with independent flattened-score controls."""
import json,struct,hashlib
from pathlib import Path
from generate_xm_key_off_regressions import generate as immediate,cell
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/xm-jump-break'
def generate():
    authored=immediate();base=authored['key-off.xm'];at=60+struct.unpack_from('<I',base,60)[0];body=base[at+9+struct.unpack_from('<H',base,at+7)[0]:];header=base[:at]
    blank=cell()
    p1=[cell(50+i,1)+blank+blank for i in range(5)]+[cell(97)+blank+blank]
    p2=[cell(60+i,1)+blank+blank for i in range(3)]+[cell(97)+blank+blank]
    def module(patterns,orders):
        head=bytearray(header);struct.pack_into('<H',head,64,len(orders));struct.pack_into('<H',head,68,3);struct.pack_into('<H',head,70,len(patterns));head[80:336]=bytes(orders)+bytes(256-len(orders))
        encoded=[]
        for rows in patterns:
            data=b''.join(rows);encoded.append(struct.pack('<IBHH',9,0,len(rows),len(data))+data)
        return bytes(head)+b''.join(encoded)+body
    files={'video.mkv':authored['video.mkv']};cases=[]
    specs=[('jump-then-break',[(11,2),(13,2),(0,0)],2,2,True),('break-then-jump',[(13,2),(11,2),(0,0)],2,0,False),('jump-two-breaks',[(11,2),(13,3),(13,2)],2,2,True),('jump-large-break',[(11,2),(13,0x64),(0,0)],2,0,True),('jump-only',[(11,2),(0,0),(0,0)],2,0,False),('break-only',[(13,2),(0,0),(0,0)],1,2,False)]
    for name,effects,order,resume,fails in specs:
        first=b''.join(cell(49 if i==0 else 0,1 if i==0 else 0,effect=e,parameter=p) for i,(e,p) in enumerate(effects))
        selected=(p1[resume:]+p2) if order==1 else p2[resume:]
        control=[cell(49,1)+blank+blank]+selected
        timeline=[[0,0]]+([[1,i] for i in range(resume,len(p1))]+[[2,i] for i in range(len(p2))] if order==1 else [[2,i] for i in range(resume,len(p2))])
        files[name+'.xm']=module([[first],p1,p2],[0,1,2]);files[name+'-control.xm']=module([control],[0])
        cases.append(dict(name=name,file=name+'.xm',control=name+'-control.xm',timeline=timeline,before_failure=fails))
    files['manifest.json']=(json.dumps({'provenance':'Authored three-order XM navigation, literal expected order/row traces and independent flattened-score controls. Includes both channel orders,multiple breaks,large target,isolated B/D. Signed16loop and authored VP9/PCM companion. No external encoder,network or private media.','cases':cases,'sha256':{n:hashlib.sha256(b).hexdigest() for n,b in files.items()}},indent=2)+'\n').encode();return files
if __name__=='__main__':
    OUT.mkdir(parents=True,exist_ok=True)
    for n,b in generate().items():(OUT/n).write_bytes(b)
