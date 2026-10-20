"""Author literal XM F/B/D opcodes versus equivalent pre-expanded timelines."""
import json,hashlib,struct
from pathlib import Path
from generate_xm_sample_index_regressions import generate as indices
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/xm-effect-opcodes'
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
    default=[cell(49,1),cell(),cell(97)]
    for value in [3,31,32,250]:
        speed,bpm=(value,125) if value<32 else (6,value)
        rows=[cell(49,1,0x0f,value),cell(),cell(97)]
        add('tempo-'+str(value),module([rows]),module([default],speed=speed,bpm=bpm),[[0,i] for i in range(3)])
    p1=[cell(50,1),cell(),cell(97)]
    add('jump',module([[cell(49,1,0x0b,1),cell(50,1),cell(97)],p1]),module([[cell(49,1),*p1]]),[[0,0],[1,0],[1,1],[1,2]])
    p1=[cell(49,1),cell(50,1),cell(),cell(97)]
    add('break',module([[cell(49,1,0x0d,1),cell(50,1),cell(97)],p1]),module([[cell(49,1),*p1[1:]]]),[[0,0],[1,1],[1,2],[1,3]])
    for name,opcode in [('default-channel-volume',0x0c),('default-global-volume',0x10)]:
        add(name,module([[cell(49,1,opcode,64),cell(),cell(97)]]),module([default]),[[0,i] for i in range(3)],True)
    files['manifest.json']=(json.dumps(dict(provenance='Eight authored literal XM opcodes F03/F1F/F20/FFA,B01,D01 plus C40/G40default-volume no-op guards. Independent controls compile control flow/tempo into header/row order with no effects. C/Gguards do not claim arbitrary volume-effect support. Constant synthetic loop and short native VP9/PCM companion. No encoder/network/private media.',cases=cases,sha256={n:hashlib.sha256(v).hexdigest() for n,v in files.items()}),indent=2)+'\n').encode();return files
if __name__=='__main__':
    OUT.mkdir(parents=True,exist_ok=True)
    for n,b in generate().items():(OUT/n).write_bytes(b)
