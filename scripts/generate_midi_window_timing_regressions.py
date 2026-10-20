"""Author native MIDI packet timing faults and short source/video controls."""
import json,struct,hashlib
from pathlib import Path
from generate_smf_clock_range_regressions import generate as smf
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/midi-window-timing'
def record(tag,at,key=69,value=64):return struct.pack('<BIBBBH',tag,at,0,key,value,0)
def generate():
    source=smf();files={'video.mkv':source['video.mkv'],'control.mid':source['control.mid']};cases=[]
    for name,data,bad in [('normal',record(1,0)+record(2,50000),False),('equal-time',record(1,0)+record(2,0),False),('last-microsecond',record(1,99999),False),('at-end',record(1,100000),True),('after-end',record(1,100001),True),('maximum-offset',record(1,0xffffffff),True),('descending',record(1,50000)+record(2,25000),True)]:
        files[name+'.packet']=data;cases.append(dict(name=name,file=name+'.packet',bad=bad))
    files['manifest.json']=(json.dumps(dict(provenance='Authored native MIDI10byte event records for100ms decoder windows. Corrupt packet timing is injected into first packet of authored100ms note source; separate short native VP9/PCM video. No private input/encoder/network.',cases=cases,sha256={n:hashlib.sha256(v).hexdigest() for n,v in files.items()}),indent=2)+'\n').encode();return files
if __name__=='__main__':
    OUT.mkdir(parents=True,exist_ok=True)
    for n,b in generate().items():(OUT/n).write_bytes(b)
