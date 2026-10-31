#!/usr/bin/env python3
"""Authored valid EOF and deliberate decoder-refusal/epoch controls,offline."""
from pathlib import Path
import hashlib,json
from generate_mp4_seek_io_regressions import generate as source
from generate_vp9_chroma import mp4
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/player-seek-failures'
def generate():
    seed=source();packets=[seed[f'frame-{i%3}.packet'] for i in range(5)]
    files={f'frame-{i}.yuv16':seed[f'frame-{i}.yuv16'] for i in range(3)}
    files['valid.mp4']=mp4(packets,8,'420',size=(16,16),sync_samples=[1,2,3,4,5])
    # Explicitly authored invalid marker,not a mutation/fuzz campaign.
    files['bad-tail.mp4']=mp4(packets[:4]+[b'\0'],8,'420',size=(16,16),sync_samples=[1,2,3,4,5])
    from generate_mp4_frame_step import generate as sound
    audio=sound();files['sound.wav']=audio['sound.wav']
    import struct
    files['sound.f32le']=b''.join(struct.pack('<f',((i*17%1024)-512)/32768) for i in range(1600))
    files['manifest.json']=(json.dumps({'provenance':'Only authored16x16 VP9 colored keyframes,YUV oracles,and200ms mono8kHz PCM/scalar oracle. Valid complete movie and explicit one-byte invalid VP9 marker in the last packet. No private media,encoder or network.','timescale':50,'valid_coded_prefix':4,'visible_prefix':3,'bad_sample':4,'bad_target_ms':80,'expected_error':'invalid VP9 frame marker','sha256':{n:hashlib.sha256(b).hexdigest() for n,b in files.items()}},indent=2)+'\n').encode()
    return files
if __name__=='__main__':
    OUT.mkdir(parents=True,exist_ok=True)
    for n,b in generate().items():(OUT/n).write_bytes(b)
