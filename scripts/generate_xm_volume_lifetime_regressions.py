"""Author zero-volume held-note phase continuity and stopped/end controls."""
import json,hashlib,struct
from pathlib import Path
from generate_xm_sample_index_regressions import generate as indices
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/xm-volume-lifetime'
def cell(note=0,instrument=0,volume=0,effect=0,parameter=0):return bytes([note,instrument,volume,effect,parameter])
def generate():
    authored=indices();base=authored['single-zero.xm'];at=60+struct.unpack_from('<I',base,60)[0];inst=at+9+struct.unpack_from('<H',base,at+7)[0];header=base[:at];instrument=bytearray(base[inst:inst+243]);struct.pack_into('<H',instrument,239,0)
    frames=[(i-32)*512 for i in range(64)];last=0;payload=bytearray()
    for value in frames:payload+=struct.pack('<h',value-last);last=value
    def module(rows,loop=True):
        sample=bytearray(40);struct.pack_into('<III',sample,0,len(payload),0,len(payload));sample[12]=64;sample[14]=16+(1 if loop else 0);sample[15]=128
        data=b''.join(rows);return header+struct.pack('<IBHH',9,0,len(rows),len(data))+data+bytes(instrument)+bytes(sample)+bytes(payload)
    specs=[('late-mute',[cell(49,1,0x50),cell(volume=0x10),cell(volume=0x50),cell(97)],[cell(49,1,0x50),cell(),cell(),cell(97)],1,2,True,True),('initial-mute',[cell(49,1,0x10),cell(volume=0x50),cell(97)],[cell(49,1,0x50),cell(),cell(97)],0,1,True,True),('released-control',[cell(49,1,0x50),cell(97),cell(volume=0x50)],[cell(49,1,0x50),cell(effect=0x0C),cell(volume=0x50)],1,2,True,True),('unlooped-end-control',[cell(49,1,0x10),cell(volume=0x50),cell(97)],[cell(49,1,0x50),cell(),cell(97)],0,1,False,False)]
    files={'video.mkv':authored['video.mkv']};cases=[]
    for name,actual,control,mute,resume,loop,audible in specs:
        files[name+'.xm']=module(actual,loop);files[name+'-continuous.xm']=module(control,loop);cases.append(dict(name=name,file=name+'.xm',control=name+'-continuous.xm',mute=mute,resume=resume,audible_after=audible))
    files['manifest.json']=(json.dumps(dict(provenance='Four authored XM signed16ramp samples with supported volume-column mute/unmute. Late/initial mute require continuous sample phase after volume restores. No-envelope keyoff followed by explicit volume restore must resume the continuing phase,against independent C00/volume-column restore;unlooped natural-end control stays silent. Native smoothing first64frames are excluded from independent steady-phase oracle. Short authored VP9/PCM companion. No encoder/network/private media.',cases=cases,sha256={n:hashlib.sha256(v).hexdigest() for n,v in files.items()}),indent=2)+'\n').encode();return files
if __name__=='__main__':
    OUT.mkdir(parents=True,exist_ok=True)
    for n,b in generate().items():(OUT/n).write_bytes(b)
