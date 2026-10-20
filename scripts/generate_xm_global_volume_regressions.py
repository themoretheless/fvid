"""Author XM global-volume gain/ordering/headroom tests independently of the decoder."""
import json,hashlib,struct
from pathlib import Path
from generate_xm_volume_lifetime_regressions import generate as lifetime
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/xm-global-volume'
def cell(note=0,instrument=0,volume=0,effect=0,parameter=0):return bytes([note,instrument,volume,effect,parameter])
def generate():
    authored=lifetime();base=authored['late-mute.xm'];at=60+struct.unpack_from('<I',base,60)[0];body=base[at+9+struct.unpack_from('<H',base,at+7)[0]:]
    def module(rows):
        channels=len(rows[0]);header=bytearray(base[:at]);struct.pack_into('<H',header,68,channels);data=b''.join(b''.join(row) for row in rows);assert all(len(row)==channels for row in rows)
        return bytes(header)+struct.pack('<IBHH',9,0,len(rows),len(data))+data+body
    files={'video.mkv':authored['video.mkv']};cases=[]
    def add(name,rows,gains,seek=1,oracle='linear',control=None):
        without=[[c[:3]+b'\0\0' if c[3]==0x10 else c for c in row] for row in rows]
        files[name+'.xm']=module(rows);files[name+'-reference.xm']=module(without if control is None else control);cases.append(dict(name=name,file=name+'.xm',control=name+'-reference.xm',gains=gains,seek=seek,oracle=oracle))
    for value in [0,1,16,32,64,255]:add('level-'+str(value),[[cell(49,1,effect=0x10,parameter=value)],[cell()],[cell(97)]],[min(value,64)]*4)
    add('persist',[[cell(49,1)],[cell(effect=0x10,parameter=32)],[cell()],[cell(97)]],[64,32,32,32,32],2)
    add('recover',[[cell(49,1)],[cell(effect=0x10,parameter=0)],[cell(effect=0x10,parameter=64)],[cell(97)]],[64,0,64,64,64],2)
    add('all-channels',[[cell(49,1),cell(50,1,effect=0x10,parameter=32)],[cell(),cell()],[cell(97),cell(97)]],[32]*4)
    add('last-command',[[cell(49,1,effect=0x10,parameter=32),cell(50,1,effect=0x10,parameter=16)],[cell(),cell()],[cell(97),cell(97)]],[16]*4)
    rows=[[cell(49,1) for _ in range(32)],[cell() for _ in range(32)],[cell(97) for _ in range(32)]];rows[0][-1]=cell(49,1,effect=0x10,parameter=16)
    control=[[cell(49,1,volume=0x20) for _ in range(32)],[cell() for _ in range(32)],[cell(97) for _ in range(32)]]
    add('preclip-headroom',rows,[16]*4,oracle='column-steady',control=control)
    files['manifest.json']=(json.dumps(dict(provenance='Eleven authored literal XM Gxxcases:0/1/16/32/64/255clamped,sticky state,zero/recover,all-channel scope,last command precedence and32channel pre-clamp headroom. Ordinary cases compare output to gain-scaled effect-free native reference; clipping case uses independent per-channel quarter-volume encoding beyond64smoothing frames. Signed16ramp loops,no other unsupported features. Short authored VP9/PCM companion. No encoder/network/private media.',cases=cases,sha256={n:hashlib.sha256(v).hexdigest() for n,v in files.items()}),indent=2)+'\n').encode();return files
if __name__=='__main__':
    OUT.mkdir(parents=True,exist_ok=True)
    for n,b in generate().items():(OUT/n).write_bytes(b)
