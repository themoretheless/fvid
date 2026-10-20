"""Author MIDI channel-message data-bit controls and malformed operands."""
import json,hashlib
from pathlib import Path
from generate_smf_clock_range_regressions import midi
from generate_audio_reader_eof_regressions import generate as pcm
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/smf-channel-data'
def generate():
    files={'video.mkv':pcm()['video.mkv']};cases=[]
    def add(name,message,bad):
        files[name+'.mid']=midi(b'\0\x90\x45\x40'+b'\0'+message+b'\x60\x80\x45\0\0\xff\x2f\0',480);cases.append(dict(name=name,file=name+'.mid',bad=bad))
    for status in [0x80,0x90,0xa0,0xb0,0xc0,0xd0,0xe0]:
        width=1 if status in [0xc0,0xd0] else 2
        for value in [0,127]:add(f'valid-{status:02x}-{value}',bytes([status])+bytes([value])*width,False)
        for operand in range(width):
            for value in [128,255]:
                args=[64]*width;args[operand]=value;add(f'bad-{status:02x}-{operand}-{value}',bytes([status]+args),True)
    add('running-valid',bytes([64,127]),False)
    for value in [128,255]:add(f'running-bad-{value}',bytes([64,value]),True)
    files['manifest.json']=(json.dumps(dict(provenance='Authored100ms MIDI note/control messages: seven channel voice status classes,7bit boundaries and status-bit operand corruption, including running status. Separate short VP9/PCM video. No encoder/private media/network.',cases=cases,sha256={n:hashlib.sha256(v).hexdigest() for n,v in files.items()}),indent=2)+'\n').encode();return files
if __name__=='__main__':
    OUT.mkdir(parents=True,exist_ok=True)
    for n,b in generate().items():(OUT/n).write_bytes(b)
