"""Author XM signed finetune scale."""
import json,hashlib,struct
from pathlib import Path
from generate_xm_volume_lifetime_regressions import generate as lifetime
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/xm-finetune'
def cell(note=0,instrument=0,volume=0,effect=0,parameter=0):return bytes([note,instrument,volume,effect,parameter])
def generate():
    authored=lifetime();base=authored['late-mute.xm'];at=60+struct.unpack_from('<I',base,60)[0];body=base[at+9+struct.unpack_from('<H',base,at+7)[0]:];header=base[:at]
    def module(rows):
        data=b''.join(rows);return header+struct.pack('<IBHH',9,0,len(rows),len(data))+data+body
    files={'video.mkv':authored['video.mkv']};cases=[]
    rows=[cell(49,1),cell(),cell(97)]
    for value in [-128,-64,-16,-8,-1,0,1,7,8,15,16,64,127]:
        data=bytearray(module(rows));sample=at+9+len(rows)*5+243;data[sample+13]=value%256
        name='fine-'+str(value);files[name+'.xm']=bytes(data);cases.append(dict(name=name,file=name+'.xm',finetune=value))
    files['minus-semitone-control.xm']=module([cell(48,1),cell(),cell(97)])
    for source in ['fine--128.xm','minus-semitone-control.xm']:
        data=bytearray(files[source]);struct.pack_into('<H',data,74,0);files['amiga-'+source]=bytes(data)
    files['manifest.json']=(json.dumps(dict(provenance='Thirteen authored signed raw finetune bytes with looped signed16ramp,note49andexplicit release. Independent pitch oracle uses signed finetune/128semitones from original XM formula. Short authored VP9/PCM companion. No private media/encoder/network.',cases=cases,sha256={n:hashlib.sha256(v).hexdigest() for n,v in files.items()}),indent=2)+'\n').encode();return files
if __name__=='__main__':
    OUT.mkdir(parents=True,exist_ok=True)
    for n,b in generate().items():(OUT/n).write_bytes(b)
