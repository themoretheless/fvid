#!/usr/bin/env python3
"""Authored finite XM E6x loops and literal flattened-score controls."""
import json,struct,hashlib
from pathlib import Path
from generate_xm_key_off_regressions import generate as immediate,cell
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/xm-pattern-loop'
def generate():
    authored=immediate();base=authored['key-off.xm'];at=60+struct.unpack_from('<I',base,60)[0];body=base[at+9+struct.unpack_from('<H',base,at+7)[0]:];header=base[:at]
    def row(note=0,effect=0,param=0,note2=0,effect2=0,param2=0):
        return cell(note,1 if 1<=note<=96 else 0,effect=effect,parameter=param)+cell(note2,1 if 1<=note2<=96 else 0,effect=effect2,parameter=param2)
    def module(patterns):
        head=bytearray(header);struct.pack_into('<H',head,64,len(patterns));struct.pack_into('<H',head,68,2);struct.pack_into('<H',head,70,len(patterns));head[80:336]=bytes(range(len(patterns)))+bytes(256-len(patterns));encoded=[]
        for rows in patterns:
            data=b''.join(rows);encoded.append(struct.pack('<IBHH',9,0,len(rows),len(data))+data)
        return bytes(head)+b''.join(encoded)+body
    def strip_flow(data):
        out=bytearray(data)
        for at in range(0,len(out),5):
            if out[at+3] in [11,13] or (out[at+3]==14 and out[at+4]>>4==6):out[at+3:at+5]=bytes(2)
        return bytes(out)
    specs=[]
    def one(name,rows,trace,bad=True):specs.append((name,[rows],[(0,i)for i in trace],bad,True))
    one('start-zero-one',[row(49,14,0x60),row(50,14,0x61),row(97)],[0,1]*2+[2])
    one('start-one-two',[row(49),row(50,14,0x60),row(51,14,0x62),row(97)],[0]+[1,2]*3+[3])
    one('implicit-start',[row(49),row(50,14,0x62),row(97)],[0,1]*3+[2])
    one('maximum-repeat',[row(49,14,0x60),row(50,14,0x6f),row(97)],[0,1]*16+[2])
    one('marker-only',[row(49,14,0x60),row(97)],[0,1],False)
    p0=[row(49),row(50,14,0x60),row(51,14,0x61),row(97)];p1=[row(60),row(61),row(97)]
    specs.append(('next-pattern-row-quirk',[p0,p1],[(0,i)for i in [0,1,2,1,2,3]]+[(1,1),(1,2)],True,True))
    p0=[row(49),row(50),row(51,14,0x60),row(52,14,0x61),row(97)];p1=[row(60),row(97)]
    specs.append(('next-short-fallback',[p0,p1],[(0,i)for i in [0,1,2,3,2,3,4]]+[(1,0),(1,1)],True,True))
    p0=[row(49,14,0x60),row(50,14,0x61,effect2=11,param2=2)];middle=[row(55),row(97)];last=[row(65),row(97)]
    specs.append(('jump-overrides-loop',[p0,middle,last],[(0,0),(0,1),(2,0),(2,1)],False,True))
    p0=[row(49),row(50,effect2=14,param2=0x60),row(51,11,2,effect2=14,param2=0x61)]
    specs.append(('loop-after-jump',[p0,middle,last],[(0,0),(0,1),(0,2),(2,1)],True,True))
    one('channel-local-counters',[row(49,14,0x60,50,14,0x60),row(51,14,0x61,52,14,0x62),row(97,note2=97)],[0,1]*6+[2])
    one('global-backward-jump',[row(49,14,0x60),row(50,14,0x61),row(97,11,0)],[0,1]*2+[2])
    one('tempo-inside-loop',[row(49,14,0x60,effect2=15,param2=6),row(50,14,0x61,effect2=15,param2=3),row(97,effect2=15,param2=6)],[0,1]*2+[2])
    p0=[row(49,14,0x60),row(50,14,0x63,effect2=11,param2=1)];p1=[row(60,14,0x61),row(97)]
    specs.append(('counter-persists-across-orders',[p0,p1],[(0,0),(0,1)]+[(1,0)]*3+[(1,1)],True,True))
    specs.append(('no-loop-control',[[row(49),row(50),row(97)]],[(0,0),(0,1),(0,2)],False,False))
    files={'video.mkv':authored['video.mkv']};cases=[]
    for name,patterns,trace,bad,loop in specs:
        files[name+'.xm']=module(patterns);files[name+'-expanded.xm']=module([[strip_flow(patterns[o][r])for o,r in trace]])
        cases.append(dict(name=name,file=name+'.xm',control=name+'-expanded.xm',timeline=trace,before_failure=bad,has_loop=loop))
    files['manifest.json']=(json.dumps({'provenance':'Authored E6x finite loops with literal expected traces and independent flattened scores. Marker/default starts,1/2/15repeats,channel-local counters,FT2next-pattern-row quirk and short fallback,position-jump precedence,global one-pass policy,tempo replay and cross-order counter lifetime. Signed16loop and VP9/PCM companion. No external encoder,network or private media.','cases':cases,'sha256':{n:hashlib.sha256(b).hexdigest() for n,b in files.items()}},indent=2)+'\n').encode();return files
if __name__=='__main__':
    OUT.mkdir(parents=True,exist_ok=True)
    for n,b in generate().items():(OUT/n).write_bytes(b)
