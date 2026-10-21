#!/usr/bin/env python3
"""Author equal-row tempo ratios and independent Kxx/ECx deadlines."""
import hashlib
import json
import struct
from pathlib import Path
from generate_xm_key_off_regressions import generate as owned,cell
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/xm-tick-span'

def generate():
    original=owned();base=original['key-off.xm'];pattern=60+struct.unpack_from('<I',base,60)[0]
    header=base[:pattern];body=base[pattern+9+struct.unpack_from('<H',base,pattern+7)[0]:]
    def module(rows,bpm,speed):
        h=bytearray(header);struct.pack_into('<HH',h,76,speed,bpm);data=b''.join(rows)
        return bytes(h)+struct.pack('<IBHH',9,0,len(rows),len(data))+data+body
    files={'video.mkv':original['video.mkv']};cases=[]
    ordinary=[cell(49,1)]+[cell()]*8+[cell(97)]
    def add(name,bpm,speed,ref_bpm,ref_speed,rows,control,score_us,last_us,bad=True,kind='row'):
        files[name+'.xm']=module(rows,bpm,speed);files[name+'-control.xm']=module(control,ref_bpm,ref_speed)
        cases.append(dict(name=name,file=name+'.xm',control=name+'-control.xm',before_failure=bad,kind=kind,score_us=score_us,duration_ms=(score_us+last_us)//1000,post_row=1 if kind=='deadline' else 2))
    for bpm,speed,rb,rs,row_us in [(150,6,125,5,100000),(175,7,125,5,100000),(225,9,125,5,100000),(120,6,100,5,125000)]:
        add(f'row-{bpm}-{speed}',bpm,speed,rb,rs,ordinary,ordinary,row_us*10,row_us)
    rows=ordinary.copy();rows[0]=cell(49,1,effect=15,parameter=150)
    control=ordinary.copy();control[0]=cell(49,1,effect=15,parameter=125)
    add('tempo-change',125,6,125,5,rows,control,1000000,100000)
    rows=ordinary.copy();rows[2]=cell(effect=15,parameter=12)
    control=ordinary.copy();control[2]=cell(effect=15,parameter=10)
    add('speed-change',150,6,125,5,rows,control,1800000,200000)
    for name,effect,param,ref_param in [('key-off-half-row',0x14,3,2),('note-cut-half-row',14,0xC3,0xC2)]:
        rows=[cell(49,1,effect=effect,parameter=param),cell(volume=0x50),cell(97),cell()]
        control=rows.copy();control[0]=cell(49,1,effect=effect,parameter=ref_param)
        add(name,150,6,100,4,rows,control,400000,100000,kind='deadline')
    add('exact-125-control',125,6,125,6,ordinary,ordinary,1200000,120000,bad=False,kind='healthy')
    add('exact-100-control',100,5,100,5,ordinary,ordinary,1250000,125000,bad=False,kind='healthy')
    files['manifest.json']=(json.dumps(dict(provenance='Ten authored XM tempo-ratio scores. Independent controls use exact whole-tick periods at BPM100/125. Equal speed/BPM ratios must keep row times identical. K03/EC3 at150BPM and K02/EC2 at100BPM both target50ms. A fixed100ms decoder window isolates deadline rounding from row duration. Tail length follows the final row. Owned signed16 ramp and VP9/PCM companion only;no encoder,network or private media.',cases=cases,sha256={n:hashlib.sha256(b).hexdigest() for n,b in files.items()}),indent=2)+'\n').encode();return files
if __name__=='__main__':
    OUT.mkdir(parents=True,exist_ok=True)
    for n,b in generate().items():(OUT/n).write_bytes(b)
