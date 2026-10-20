"""Author XM transposed pitch range."""
import json,hashlib,struct
from pathlib import Path
from generate_xm_volume_lifetime_regressions import generate as lifetime
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/xm-relative-pitch'
def cell(note=0,instrument=0,volume=0,effect=0,parameter=0):return bytes([note,instrument,volume,effect,parameter])
def generate():
    authored=lifetime();base=authored['late-mute.xm'];at=60+struct.unpack_from('<I',base,60)[0];body=base[at+9+struct.unpack_from('<H',base,at+7)[0]:];header=base[:at]
    def module(rows):
        data=b''.join(rows);return header+struct.pack('<IBHH',9,0,len(rows),len(data))+data+body
    files={'video.mkv':authored['video.mkv']};cases=[]
    for note,relative in [(84,12),(96,1),(96,12),(96,23),(96,24),(1,-1)]:
        rows=[cell(note,1),cell(),cell(97)];data=bytearray(module(rows));sample=at+9+len(rows)*5+243;data[sample+16]=relative%256
        name=f'note-{note}-relative-{relative}';files[name+'.xm']=bytes(data);cases.append(dict(name=name,file=name+'.xm',note=note,relative=relative,audible=1<=note+relative<=119))
    files['manifest.json']=(json.dumps(dict(provenance='Six authored pattern-note plus sample-relative-tone controls. Totalpitch96healthy,97/108/119valid high registers,0/120explicit unsupported bounds. Signed16ramp loop andindependent originalXMlinear pitch waveform oracle;short authored VP9/PCM companion. No private media/encoder/network.',cases=cases,sha256={n:hashlib.sha256(v).hexdigest() for n,v in files.items()}),indent=2)+'\n').encode();return files
if __name__=='__main__':
    OUT.mkdir(parents=True,exist_ok=True)
    for n,b in generate().items():(OUT/n).write_bytes(b)
