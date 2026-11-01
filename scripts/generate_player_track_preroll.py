#!/usr/bin/env python3
"""Authored valid MP4 for actual input-wait/track-replacement gating;offline."""
from pathlib import Path
import hashlib,json
from generate_player_track_clock import generate as source
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/player-track-preroll'
def generate():
    files={n:b for n,b in source().items()if n!='manifest.json'}
    files['manifest.json']=(json.dumps({'provenance':'Authored1.5s five-frame16x16 VP9 MP4 with two distinct600ms mono8kHz PCM tracks and scalar YUV/F32 oracles. Actual one-byte input gates sample2 after the first video frame;ordinary volume-command acknowledgments observe backend running state. No private media,FFmpeg,encoder or network.','video_ms':1500,'audio_ms':600,'seek_ms':450,'gated_video_sample':2,'sha256':{n:hashlib.sha256(b).hexdigest()for n,b in files.items()}},indent=2)+'\n').encode()
    return files
if __name__=='__main__':
    OUT.mkdir(parents=True,exist_ok=True)
    for n,b in generate().items():(OUT/n).write_bytes(b)
