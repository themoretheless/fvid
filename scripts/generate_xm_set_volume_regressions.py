"""Author XM Cxx versus independent supported volume-column controls."""
import json,hashlib,struct
from pathlib import Path
from generate_xm_volume_lifetime_regressions import generate as lifetime
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/xm-set-volume'
def cell(note=0,instrument=0,volume=0,effect=0,parameter=0):return bytes([note,instrument,volume,effect,parameter])
def generate():
    authored=lifetime();base=authored['late-mute.xm'];at=60+struct.unpack_from('<I',base,60)[0];body=base[at+9+struct.unpack_from('<H',base,at+7)[0]:];header=base[:at]
    def module(rows):
        data=b''.join(rows);return header+struct.pack('<IBHH',9,0,len(rows),len(data))+data+body
    files={'video.mkv':authored['video.mkv']};cases=[]
    def add(name,actual,control,seek=1):
        files[name+'.xm']=module(actual);files[name+'-column.xm']=module(control)
        without=[r[:3]+b'\0\0' if r[3]==0x0c else r for r in actual];files[name+'-without.xm']=module(without)
        cases.append(dict(name=name,file=name+'.xm',control=name+'-column.xm',without=name+'-without.xm',seek=seek))
    for value in [0,1,16,32,64,255]:add('volume-'+str(value),[cell(49,1),cell(effect=0x0c,parameter=value),cell(97)],[cell(49,1),cell(volume=0x10+min(value,64)),cell(97)])
    add('override-column',[cell(49,1),cell(volume=0x50,effect=0x0c,parameter=0),cell(97)],[cell(49,1),cell(volume=0x10),cell(97)])
    add('note-on',[cell(49,1,effect=0x0c,parameter=16),cell(),cell(97)],[cell(49,1,volume=0x20),cell(),cell(97)])
    add('persist',[cell(49,1),cell(effect=0x0c,parameter=32),cell(),cell(97)],[cell(49,1),cell(volume=0x30),cell(),cell(97)],2)
    add('recover',[cell(49,1),cell(effect=0x0c,parameter=0),cell(effect=0x0c,parameter=64),cell(97)],[cell(49,1),cell(volume=0x10),cell(volume=0x50),cell(97)],2)
    files['manifest.json']=(json.dumps(dict(provenance='Ten authored literal XM Cxxcases compared with independent volume-column encoding,0/1/16/32/64/255clamped,volume-column precedence,note-on,persistence andmute/recover. Signed16ramp loop,no envelopes/fadeouts orother unsupported effects. Without-effect counterparts separate current ignored reproduction from acceptance. Short authored VP9/PCM companion. No encoder/network/private media.',cases=cases,sha256={n:hashlib.sha256(v).hexdigest() for n,v in files.items()}),indent=2)+'\n').encode();return files
if __name__=='__main__':
    OUT.mkdir(parents=True,exist_ok=True)
    for n,b in generate().items():(OUT/n).write_bytes(b)
