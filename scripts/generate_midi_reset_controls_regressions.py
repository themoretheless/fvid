"""Author CC121 versus explicit supported-controller reset sequences."""
import json,hashlib
from pathlib import Path
from generate_smf_clock_range_regressions import midi
from generate_audio_reader_eof_regressions import generate as pcm
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/midi-reset-controls'
def generate():
    files={'video.mkv':pcm()['video.mkv']};cases=[]
    specs=[('bend-up',0,16383,127,127,0,False),('bend-down',15,0,127,127,0,False),('volume-program',0,8192,31,64,43,False),('mixed-channels',0,16383,47,31,17,True),('already-default',0,8192,127,127,23,False)]
    for name,ch,bend,volume,expression,program,mixed in specs:
        def event(status,*args):return b'\0'+bytes([status+ch,*args])
        prefix=event(0xc0,program)+event(0xb0,7,volume)+event(0xb0,11,expression)+event(0xe0,bend&127,bend>>7)+event(0x90,69,100)
        if mixed:prefix+=b'\0\xe7\0\0\0\x97\x4c\x50'
        suffix=b'\x60'+bytes([0x80+ch,69,0])+(b'\0\x87\x4c\0' if mixed else b'')+b'\0\xff\x2f\0'
        reset=b'\x60'+bytes([0xb0+ch,121,0]);explicit=b'\x60'+bytes([0xb0+ch,11,127])+event(0xe0,0,64)
        files[name+'.mid']=midi(prefix+reset+suffix,480);files[name+'-explicit.mid']=midi(prefix+explicit+suffix,480)
        cases.append(dict(name=name,file=name+'.mid',control=name+'-explicit.mid',channel=ch,program=program))
    files['manifest.json']=(json.dumps(dict(provenance='Five authored200ms MIDI notes compare ResetAllControllers121 with explicit expression127 and centered pitch bend while preserving channel volume/program. Up/down bends,channel15,nondefault volume/program,mixed channel isolation and unchanged-state control. Separate authored native VP9/PCM video. No encoder/network/private media.',cases=cases,sha256={n:hashlib.sha256(v).hexdigest() for n,v in files.items()}),indent=2)+'\n').encode();return files
if __name__=='__main__':
    OUT.mkdir(parents=True,exist_ok=True)
    for n,b in generate().items():(OUT/n).write_bytes(b)
