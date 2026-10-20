"""Author native MIDI record operand faults, boundaries and ignored-byte controls."""
import json,hashlib,struct
from pathlib import Path
from generate_midi_window_timing_regressions import generate as timing
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/midi-record-operands'
def record(tag,first=0,second=0,wide=0):return struct.pack('<BIBBBH',tag,0,0,first,second,wide)
def generate():
    source=timing();files={'video.mkv':source['video.mkv'],'control.mid':source['control.mid'],'normal.packet':record(1,69,64)};cases=[]
    def add(name,data,bad):files[name+'.packet']=data;cases.append(dict(name=name,file=name+'.packet',bad=bad))
    for value in [128,255]:
        for label,tag,first,second in [('note-key',1,value,64),('note-velocity',1,69,value),('off-key',2,value,0),('program',3,value,0),('control-number',4,value,64),('control-value',4,7,value)]:add(f'{label}-{value}',record(tag,first,second),True)
    for value in [16384,65535]:add('bend-'+str(value),record(5,wide=value),True)
    for key,velocity in [(0,0),(0,127),(127,0),(127,127)]:add(f'valid-note-{key}-{velocity}',record(1,key,velocity),False)
    for key in [0,127]:add('valid-off-'+str(key),record(2,key),False)
    for value in [0,127]:add('valid-program-'+str(value),record(3,value),False)
    for number,value in [(0,0),(127,127)]:add(f'valid-control-{number}-{value}',record(4,number,value),False)
    for value in [0,16383]:add('valid-bend-'+str(value),record(5,wide=value),False)
    add('valid-off-unused-bytes',record(2,69,255,65535),False)
    add('valid-bend-unused-bytes',record(5,255,255,8192),False)
    files['manifest.json']=(json.dumps(dict(provenance='Fourteen active-operand range violations inpublic nativeMIDI10byte records;fourteen7/14bit boundary/unusedbyte controls. Faults injected intofirstpacket ofauthored100ms MIDI;separate nativeVP9/PCMvideo. No encoder/network/private media.',cases=cases,sha256={n:hashlib.sha256(v).hexdigest() for n,v in files.items()}),indent=2)+'\n').encode();return files
if __name__=='__main__':
    OUT.mkdir(parents=True,exist_ok=True)
    for n,b in generate().items():(OUT/n).write_bytes(b)
