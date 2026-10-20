"""Author XM pattern-break boundaries versus equivalent pre-expanded timelines."""
import json,hashlib,struct
from pathlib import Path
from generate_xm_sample_index_regressions import generate as indices
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/xm-break-boundaries'
def cell(note=0,instrument=0,effect=0,parameter=0):return bytes([note,instrument,0,effect,parameter])
def generate():
    authored=indices();base=authored['single-zero.xm'];at=60+struct.unpack_from('<I',base,60)[0];instrument=at+9+struct.unpack_from('<H',base,at+7)[0];body=base[instrument:]
    def module(patterns,orders=None,speed=6,bpm=125):
        orders=list(range(len(patterns))) if orders is None else orders;header=bytearray(base[:at]);struct.pack_into('<H',header,64,len(orders));struct.pack_into('<H',header,70,len(patterns));struct.pack_into('<HH',header,76,speed,bpm);header[80:336]=bytes(256);header[80:80+len(orders)]=bytes(orders)
        chunks=[]
        for rows in patterns:
            data=b''.join(rows);chunks.append(struct.pack('<IBHH',9,0,len(rows),len(data))+data)
        return bytes(header)+b''.join(chunks)+body
    files={'video.mkv':authored['video.mkv']};cases=[]
    def add(name,actual,control,paths,limited=False):
        files[name+'.xm']=actual;files[name+'-expanded.xm']=control;cases.append(dict(name=name,file=name+'.xm',control=name+'-expanded.xm',paths=paths,limited_default_guard=limited))
    p1=[cell(50,1)]+[cell() for _ in range(63)]+[cell(97)]
    for value in [0,0x63,0x64,0x99,0xFF]:
        decoded=(value>>4)*10+(value&15);resume=decoded if decoded<=63 else 0
        add('break-'+str(value),module([[cell(49,1,0x0D,value),cell(97)],p1]),module([[cell(49,1),*p1[resume:]]]),[[0,0]]+[[1,i] for i in range(resume,len(p1))])
    files['manifest.json']=(json.dumps(dict(provenance='Five authored D00/D63/D64/D99/DFF controls: FT2 decodes BCD and resets values above63to row0. Independent expanded row sequences,constant synthetic PCM and native VP9/PCM companion. No private media or encoder.',cases=cases,sha256={n:hashlib.sha256(v).hexdigest() for n,v in files.items()}),indent=2)+'\n').encode();return files
if __name__=='__main__':
    OUT.mkdir(parents=True,exist_ok=True)
    for n,b in generate().items():(OUT/n).write_bytes(b)
