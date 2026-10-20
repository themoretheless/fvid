"""Authored native PCM packets for strict playback admission."""
import hashlib,json,struct
from pathlib import Path
from generate_audio_ring_capacity_regressions import clip
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/pcm-packet-admission'
def generate():
    files={}
    cases=[('valid-int',False,None),('short-byte',False,lambda raw:raw[:-1]),('short-channel',False,lambda raw:raw[:-2]),('valid-float',True,None),('nan',True,lambda raw:struct.pack('<I',0x7fc00000)+raw[4:]),('infinity',True,lambda raw:struct.pack('<I',0x7f800000)+raw[4:])]
    for name,floating,transform in cases:
        media,pcm=clip(480,8000,3,bits=32 if floating else 16,floating=floating,channels=2,payload_transform=transform)
        files[name+'.mkv']=media
        if transform is None:files[name+'-expected.f32le']=pcm
    files['manifest.json']=(json.dumps({'provenance':'Authored60ms VP9 and stereo PCM. Complete finite controls, truncated frame companions and IEEE non-finite companions verify strict playback admission. No encoder/private media/network.','sha256':{k:hashlib.sha256(v).hexdigest() for k,v in files.items()}},indent=2)+'\n').encode()
    return files
if __name__=='__main__':
    OUT.mkdir(parents=True,exist_ok=True)
    for name,data in generate().items():(OUT/name).write_bytes(data)
