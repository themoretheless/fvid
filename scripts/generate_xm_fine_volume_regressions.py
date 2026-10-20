"""Author XM fine volume slides versus independent supported volume-column controls."""
import json,hashlib,struct
from pathlib import Path
from generate_xm_volume_lifetime_regressions import generate as lifetime
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/xm-fine-volume'
def cell(note=0,instrument=0,volume=0,effect=0,parameter=0):return bytes([note,instrument,volume,effect,parameter])
def generate():
    authored=lifetime();base=authored['late-mute.xm'];at=60+struct.unpack_from('<I',base,60)[0];body=base[at+9+struct.unpack_from('<H',base,at+7)[0]:];header=base[:at]
    def module(rows):
        data=b''.join(rows);return header+struct.pack('<IBHH',9,0,len(rows),len(data))+data+body
    files={'video.mkv':authored['video.mkv']};cases=[]
    def add(name,actual,control,seek=1):
        files[name+'.xm']=module(actual);files[name+'-column.xm']=module(control)
        without=[r[:2]+b'\0'+r[3:] if 0x80<=r[2]<=0x9F else r for r in actual];files[name+'-without.xm']=module(without)
        cases.append(dict(name=name,file=name+'.xm',control=name+'-column.xm',without=name+'-without.xm',seek=seek))
    for direction,opcode in [('down',0x80),('up',0x90)]:
        for initial,amount in [(32,1),(32,15),(0,15),(64,15),(32,0)]:
            target=max(0,initial-amount) if direction=='down' else min(64,initial+amount)
            add(direction+'-'+str(initial)+'-'+str(amount),[cell(49,1,0x10+initial),cell(volume=opcode+amount),cell(),cell(97)],[cell(49,1,0x10+initial),cell(volume=0x10+target),cell(),cell(97)],2)
    add('standard-volume-overrides',[cell(49,1,0x30),cell(volume=0x9F,effect=0x0C,parameter=16),cell(97)],[cell(49,1,0x30),cell(volume=0x20),cell(97)])
    files['manifest.json']=(json.dumps(dict(provenance='Eleven authored volume-column fine-slide controls,8xdown/9xup,zero command,clamp0/64,persistence and standardCxxoverride. Independent direct volume-column assignments and signed16ramp loop;no other unsupported features. Authored VP9/PCM companion. No encoder/network/private media.',cases=cases,sha256={n:hashlib.sha256(v).hexdigest() for n,v in files.items()}),indent=2)+'\n').encode();return files
if __name__=='__main__':
    OUT.mkdir(parents=True,exist_ok=True)
    for n,b in generate().items():(OUT/n).write_bytes(b)
