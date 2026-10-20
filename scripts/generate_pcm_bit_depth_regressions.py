"""Authored native PCM tracks with declared widths that must not wrap to u8."""
import hashlib,json
from pathlib import Path
from generate_audio_ring_capacity_regressions import clip
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/pcm-bit-depth-range'
def generate():
    files={}
    for name,bits,floating in [('int16',16,False),('int272',272,False),('float32',32,True),('float288',288,True)]:
        media,pcm=clip(480,8000,3,bits=bits,floating=floating)
        files[name+'.mkv']=media
        files['expected.f32le']=pcm
    files['manifest.json']=(json.dumps({'provenance':'Authored60ms VP9 with mono PCM. Valid16bit integer/32bit float controls have identical samples; declared272/288bit companions must refuse rather than wrap their widths. No encoder/private media/network.','sha256':{k:hashlib.sha256(v).hexdigest() for k,v in files.items()}},indent=2)+'\n').encode()
    return files
if __name__=='__main__':
    OUT.mkdir(parents=True,exist_ok=True)
    for name,data in generate().items():(OUT/name).write_bytes(data)
