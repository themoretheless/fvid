"""Author XM9xx controls by physically trimming independent instrument samples."""
import json,hashlib,struct
from pathlib import Path
from generate_xm_volume_lifetime_regressions import generate as lifetime
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/xm-sample-offset'
def cell(note=0,instrument=0,volume=0,effect=0,parameter=0):return bytes([note,instrument,volume,effect,parameter])
def generate():
    authored=lifetime();base=authored['late-mute.xm'];at=60+struct.unpack_from('<I',base,60)[0];start=at+9+struct.unpack_from('<H',base,at+7)[0];header=bytearray(base[:at]);struct.pack_into('<H',header,72,4);instrument=base[start:start+243]
    files={'video.mkv':authored['video.mkv']};cases=[]
    for width in [8,16]:
        # Quantized values permit identical independent8/16-bit PCM controls.
        values=[((i*17+i//256*31)%101-50)*256 for i in range(1024)]
        def inst(values):
            record=bytearray(40);struct.pack_into('<I',record,0,len(values)*(width//8));record[12]=64;record[14]=16 if width==16 else 0;record[15]=128
            last=0;data=bytearray()
            for v in values:
                v=v if width==16 else v//256;delta=v-last;last=v;data+=(delta&((1<<width)-1)).to_bytes(width//8,'little')
            return instrument+record+data
        body=b''.join(inst(v) for v in [values,values[256:],values[512:],[0]])
        def module(rows,loop=False):
            chosen=bytearray(body)
            if loop:
                struct.pack_into('<II',chosen,243+4,256*(width//8),256*(width//8));chosen[243+14]|=1
            data=b''.join(rows);return bytes(header)+struct.pack('<IBHH',9,0,len(rows),len(data))+data+chosen
        def add(name,actual,control,loop=False):
            name=f'{width}-{name}';files[name+'.xm']=module(actual,loop);files[name+'-trimmed.xm']=module(control);cases.append(dict(name=name,file=name+'.xm',control=name+'-trimmed.xm',seek=1))
        add('offset-one',[cell(49,1,effect=9,parameter=1),cell(),cell(97)],[cell(49,2),cell(),cell(97)])
        add('offset-two',[cell(49,1,effect=9,parameter=2),cell(),cell(97)],[cell(49,3),cell(),cell(97)])
        add('zero-memory',[cell(49,1,effect=9),cell(),cell(97)],[cell(49,1),cell(),cell(97)])
        add('at-end',[cell(49,1,effect=9,parameter=4),cell(),cell(97)],[cell(49,4),cell(),cell(97)])
        add('beyond-end',[cell(49,1,effect=9,parameter=255),cell(),cell(97)],[cell(49,4),cell(),cell(97)])
        add('loop-end',[cell(49,1,effect=9,parameter=2),cell(),cell(97)],[cell(49,4),cell(),cell(97)],True)
        add('remember',[cell(49,1,effect=9,parameter=1),cell(49,1,effect=9),cell(97)],[cell(49,2),cell(49,2),cell(97)])
        add('plain-retrigger',[cell(49,1,effect=9,parameter=1),cell(49,1),cell(97)],[cell(49,2),cell(49,1),cell(97)])
        add('memory-after-plain',[cell(49,1,effect=9,parameter=1),cell(49,1),cell(49,1,effect=9),cell(97)],[cell(49,2),cell(49,1),cell(49,2),cell(97)])
        add('no-note-no-memory',[cell(49,1),cell(effect=9,parameter=2),cell(49,1,effect=9),cell(97)],[cell(49,1),cell(),cell(49,1),cell(97)])
        add('instrument-only-no-memory',[cell(49,1),cell(instrument=1,effect=9,parameter=2),cell(49,1,effect=9),cell(97)],[cell(49,1),cell(instrument=1),cell(49,1),cell(97)])
    files['manifest.json']=(json.dumps(dict(provenance='Twenty-two authored8/16bit XM9xx controls, independent physicallytrimmed instruments,quantized ramps including explicit loop-end refusal of playback beyond active loop. Offset memory,plain retrigger,no-note/instrument-only,zero andout-of-range positions. FT2position counts sampleframes atbothwidths. Separate nativevideo companion;no encoder/network/private media.',cases=cases,sha256={n:hashlib.sha256(v).hexdigest() for n,v in files.items()}),indent=2)+'\n').encode();return files
if __name__=='__main__':
    OUT.mkdir(parents=True,exist_ok=True)
    for n,b in generate().items():(OUT/n).write_bytes(b)
