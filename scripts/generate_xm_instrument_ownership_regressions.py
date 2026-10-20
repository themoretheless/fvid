#!/usr/bin/env python3
"""Authored selected-versus-playing XM instruments with independent clone controls."""
import json,struct,hashlib
from pathlib import Path
from generate_xm_volume_lifetime_regressions import generate as lifetime
from generate_xm_key_off_regressions import cell
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/xm-instrument-ownership'
def generate():
    authored=lifetime();base=authored['late-mute.xm'];at=60+struct.unpack_from('<I',base,60)[0];inst=at+9+struct.unpack_from('<H',base,at+7)[0];header=bytearray(base[:at]);struct.pack_into('<H',header,72,3)
    size=struct.unpack_from('<I',base,inst)[0];ih=base[inst:inst+size];sample=base[inst+size:inst+size+40];length=struct.unpack_from('<I',sample)[0];payload=base[inst+size+40:inst+size+40+length];first=ih+sample+payload
    second_header=bytearray(ih);second_header[4:26]=b'future B'+bytes(14);second_sample=bytearray(sample);second_sample[12]=32;second_sample[15]=224;second_sample[16]=12
    second_payload=bytearray();previous=0
    for i in range(64):
        value=(31-i)*384;second_payload+=struct.pack('<h',value-previous);previous=value
    second=bytes(second_header)+bytes(second_sample)+bytes(second_payload)
    def module(rows):
        data=b''.join(rows);return bytes(header)+struct.pack('<IBHH',9,0,len(rows),len(data))+data+first+second+first
    files={'video.mkv':authored['video.mkv']};cases=[]
    a=cell(49,1);off=cell(97);blank=cell();future=cell(50,2,0x50,8,128)
    specs=[
      ('selection-only',[a,cell(instrument=2),blank,off],[a,cell(instrument=3),blank,off],True),
      ('selection-then-note',[a,cell(instrument=2),cell(50),off],[a,cell(instrument=3),future,off],True),
      ('reset-volume',[cell(49,1,0x30),cell(instrument=2),blank,off],[cell(49,1,0x30),cell(instrument=3),blank,off],True),
      ('reset-pan',[cell(49,1,effect=8,parameter=0),cell(instrument=2),blank,off],[cell(49,1,effect=8,parameter=0),cell(instrument=3),blank,off],True),
      ('key-off-selection',[a,cell(97,2),blank,off],[a,cell(97,3),blank,off],True),
      ('k00-selection',[a,cell(50,2,effect=0x14),cell(50),off],[a,cell(50,3,effect=0x14),future,off],True),
      ('cold-selection',[cell(instrument=2),cell(49),cell(49,2),off],[blank,blank,cell(49,2),off],True),
      ('unselected-note',[cell(49),cell(instrument=2),cell(49,2),off],[blank,blank,cell(49,2),off],True),
      ('explicit-trigger-control',[a,cell(50,2),blank,off],[a,cell(50,2),blank,off],False)]
    for name,actual,control,bad in specs:
        files[name+'.xm']=module(actual);files[name+'-control.xm']=module(control);cases.append(dict(name=name,file=name+'.xm',control=name+'-control.xm',before_failure=bad))
    files['manifest.json']=(json.dumps({'provenance':'Authored A ramp,B reversed ramp with distinct default volume/pan/transposition,and clone A. Instrument-only clone controls preserve independent active-wave PCM;future B notes explicitly restore old A level/pan in controls. Cold/unselected controls stay silent until explicit trigger. Signed16loops,no envelopes/fades,VP9/PCM companion. No external encoder,network or private media.','cases':cases,'sha256':{n:hashlib.sha256(b).hexdigest() for n,b in files.items()}},indent=2)+'\n').encode();return files
if __name__=='__main__':
    OUT.mkdir(parents=True,exist_ok=True)
    for n,b in generate().items():(OUT/n).write_bytes(b)
