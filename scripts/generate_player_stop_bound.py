#!/usr/bin/env python3
"""Short authored video/audio controls for actual terminal playback bounds."""
from pathlib import Path
import hashlib,json,struct
from generate_mp4_frame_step import generate as donor
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/player-stop-bound'
def generate():
    source=donor();cases=json.loads(source['manifest.json'])['cases'];files={n:b for n,b in source.items() if n not in ['manifest.json','sound-tail.f32le']}
    files['sound.f32le']=b''.join(struct.pack('<f',((i*17%1024)-512)/32768) for i in range(1600))
    for c in cases:c['stop_after_picture']=1
    files['manifest.json']=(json.dumps({'provenance':'Only committed authored16x16 VP9 CFR/VFR/fractional keyframes and independent YUV planes;200ms authored mono8kHz PCM waveform with scalar float oracle. No private media,encoder or network.','cases':cases,'sha256':{n:hashlib.sha256(b).hexdigest() for n,b in files.items()}},indent=2)+'\n').encode()
    return files
if __name__=='__main__':
    OUT.mkdir(parents=True,exist_ok=True)
    for n,b in generate().items():(OUT/n).write_bytes(b)
