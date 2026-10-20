"""Author sub-millisecond PCM files with nonintegral-nanosecond sample boundaries."""
import json,struct,hashlib
from pathlib import Path
from generate_wav_unsigned8_regressions import wav
from generate_audio_ring_capacity_regressions import clip
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/audio-sample-seek-clock'
def generate():
    files={};cases=[]
    for rate,frames in [(48000,1),(44100,1),(44100,2),(48000,7),(8000,1)]:
        name=f'{rate}-{frames}';data=b''.join(struct.pack('<h',4096+i*256) for i in range(frames));expected=b''.join(struct.pack('<f',(4096+i*256)/32768.)for i in range(frames));files[name+'.wav']=wav(rate,1,16,data);files[name+'.f32le']=expected;files[name+'.mkv']=clip(frames,rate,1,packets=1,payload_transform=lambda _,data=data:data)[0];cases.append(dict(name=name,rate=rate,frames=frames))
    files['manifest.json']=(json.dumps(dict(provenance='Authored1/2/7frame native PCM WAV and VP9/PCM video companions. Exact sample ticks map to floored nanosecond Duration; seek to that stored boundary must recover the original tick.8kHz integral control. No encoder/private input/network.',cases=cases,sha256={n:hashlib.sha256(v).hexdigest() for n,v in files.items()}),indent=2)+'\n').encode();return files
if __name__=='__main__':
    OUT.mkdir(parents=True,exist_ok=True)
    for n,b in generate().items():(OUT/n).write_bytes(b)
