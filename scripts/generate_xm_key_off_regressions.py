"""Author XM K00 versus independent note-off controls."""
import json,hashlib,struct
from pathlib import Path
from generate_xm_volume_lifetime_regressions import generate as lifetime
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/xm-key-off'
def cell(note=0,instrument=0,volume=0,effect=0,parameter=0):return bytes([note,instrument,volume,effect,parameter])
def generate():
    authored=lifetime();base=authored['late-mute.xm'];at=60+struct.unpack_from('<I',base,60)[0];body=base[at+9+struct.unpack_from('<H',base,at+7)[0]:];header=base[:at]
    def module(rows):
        data=b''.join(rows);return header+struct.pack('<IBHH',9,0,len(rows),len(data))+data+body
    files={'video.mkv':authored['video.mkv']};cases=[]
    def add(name,actual,control,seek=1):
        files[name+'.xm']=module(actual);files[name+'-note-off.xm']=module(control)
        without=[r[:3]+b'\0\0' if r[3]==0x14 else r for r in actual];files[name+'-without.xm']=module(without)
        cases.append(dict(name=name,file=name+'.xm',control=name+'-note-off.xm',without=name+'-without.xm',seek=seek))
    add('key-off',[cell(49,1),cell(effect=0x14),cell(),cell(97)],[cell(49,1),cell(97),cell(),cell(97)],2)
    add('silent-key-off',[cell(effect=0x14),cell(49,1),cell(97)],[cell(97),cell(49,1),cell(97)])
    add('retrigger-after',[cell(49,1),cell(effect=0x14),cell(50,1),cell(97)],[cell(49,1),cell(97),cell(50,1),cell(97)],2)
    add('instrument',[cell(49,1),cell(instrument=1,effect=0x14),cell(97)],[cell(49,1),cell(97,1),cell(97)])
    add('volume-column',[cell(49,1),cell(volume=0x30,effect=0x14),cell(97)],[cell(49,1),cell(97,volume=0x30),cell(97)])
    add('ignore-new-note',[cell(49,1),cell(50,1,effect=0x14),cell(97)],[cell(49,1),cell(97,1),cell(97)])
    files['manifest.json']=(json.dumps(dict(provenance='Six authored K00 immediate key-off inputs compared with independent note97controls. Sustained loop,silence,retrigger,instrument,volume column andsame-row newnote precedence. Signed16ramp,no envelopes/fadeouts orother unsupported features. Delayed Kxxnot qualified. Authored VP9/PCM companion. No encoder/network/private media.',cases=cases,sha256={n:hashlib.sha256(v).hexdigest() for n,v in files.items()}),indent=2)+'\n').encode();return files
if __name__=='__main__':
    OUT.mkdir(parents=True,exist_ok=True)
    for n,b in generate().items():(OUT/n).write_bytes(b)
