"""Author XM volume-column panning versus independently assigned instrument panning."""
import json,hashlib,struct
from pathlib import Path
from generate_xm_volume_lifetime_regressions import generate as lifetime
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/xm-volume-panning'
def cell(note=0,instrument=0,volume=0,effect=0,parameter=0):return bytes([note,instrument,volume,effect,parameter])
def generate():
    authored=lifetime();base=authored['late-mute.xm'];at=60+struct.unpack_from('<I',base,60)[0];body=base[at+9+struct.unpack_from('<H',base,at+7)[0]:]
    def module(rows,pan):
        header=bytearray(base[:at]);struct.pack_into('<H',header,68,len(rows[0]));struct.pack_into('<H',header,72,2);other=bytearray(body);other[243+15]=pan;data=b''.join(b''.join(row) for row in rows)
        return bytes(header)+struct.pack('<IBHH',9,0,len(rows),len(data))+data+body+bytes(other)
    files={'video.mkv':authored['video.mkv']};cases=[]
    def add(name,actual,control,pan,seek=1):
        files[name+'.xm']=module(actual,pan);files[name+'-instrument.xm']=module(control,pan);without=[[c[:2]+b'\0'+c[3:] if c[2]>=0xC0 else c for c in row] for row in actual];files[name+'-without.xm']=module(without,pan);cases.append(dict(name=name,file=name+'.xm',control=name+'-instrument.xm',without=name+'-without.xm',seek=seek))
    for pan in [0,64,128,192,240]:add('pan-'+str(pan),[[cell(49,1)],[cell(volume=0xC0+pan//16)],[cell(97)]],[[cell(49,1)],[cell(instrument=2)],[cell(97)]],pan)
    add('note-on',[[cell(49,1,volume=0xC0)],[cell()],[cell(97)]],[[cell(49,2)],[cell()],[cell(97)]],0)
    add('effect-overrides-column',[[cell(49,1)],[cell(volume=0xC0,effect=8,parameter=192)],[cell(97)]],[[cell(49,1)],[cell(instrument=2)],[cell(97)]],192)
    add('persist',[[cell(49,1)],[cell(volume=0xC0)],[cell()],[cell(97)]],[[cell(49,1)],[cell(instrument=2)],[cell()],[cell(97)]],0,2)
    add('return-center',[[cell(49,1)],[cell(volume=0xC0)],[cell(volume=0xC8)],[cell(97)]],[[cell(49,1)],[cell(instrument=2)],[cell(instrument=1)],[cell(97)]],0,2)
    add('channel-isolation',[[cell(49,1),cell(50,1)],[cell(volume=0xC0),cell()],[cell(97),cell(97)]],[[cell(49,1),cell(50,1)],[cell(instrument=2),cell()],[cell(97),cell(97)]],0)
    files['manifest.json']=(json.dumps(dict(provenance='Ten authored volume-column C0..CF cases compare with a second instrument holding identical signed16ramp PCM,rate,volume andloop butindependently assigned panning.0/64/128/192/240,onset,8xx override,persistence,return center andchannel isolation. Mono must stay unchanged; stereo follows existing instrument-pan law. No other unsupported features. Short authored VP9/PCM companion. No encoder/network/private media.',cases=cases,sha256={n:hashlib.sha256(v).hexdigest() for n,v in files.items()}),indent=2)+'\n').encode();return files
if __name__=='__main__':
    OUT.mkdir(parents=True,exist_ok=True)
    for n,b in generate().items():(OUT/n).write_bytes(b)
