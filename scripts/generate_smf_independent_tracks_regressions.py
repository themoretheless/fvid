"""Author simultaneous and independent MIDI track-topology probes."""
import json,struct,hashlib
from pathlib import Path
from generate_audio_reader_eof_regressions import generate as pcm
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/smf-independent-tracks'
def smf(fmt,tracks):return b'MThd'+struct.pack('>IHHH',6,fmt,len(tracks),480)+b''.join(b'MTrk'+struct.pack('>I',len(t))+t for t in tracks)
def generate():
    a=b'\0\x90\x45\x40\x60\x80\x45\0\0\xff\x2f\0';b=b'\0\x91\x4c\x40\x81\x40\x81\x4c\0\0\xff\x2f\0'
    files={'video.mkv':pcm()['video.mkv'],'single0.mid':smf(0,[a]),'single1.mid':smf(1,[a]),'single2.mid':smf(2,[a]),'simultaneous.mid':smf(1,[a,b]),'independent.mid':smf(2,[a,b])}
    files['manifest.json']=(json.dumps(dict(provenance='Authored100/200ms independent notes, format1 simultaneous control and format2 independent-track topology. Single-track format0/1/2 controls. Separate short VP9/PCM video. No encoder/network/private media.',sha256={n:hashlib.sha256(v).hexdigest() for n,v in files.items()}),indent=2)+'\n').encode();return files
if __name__=='__main__':
    OUT.mkdir(parents=True,exist_ok=True)
    for n,b in generate().items():(OUT/n).write_bytes(b)
