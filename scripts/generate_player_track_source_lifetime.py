#!/usr/bin/env python3
"""Author source-lifetime MP4 witnesses,including a different same-name file."""
from pathlib import Path
import hashlib,json,struct
from generate_player_track_clock import generate as source
from generate_mp4_enabled_track_regressions import atom,word,boxes
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/player-track-source-lifetime'
def generate():
    files={n:b for n,b in source().items()if n!='manifest.json'};base=dict(boxes(files['movie.mp4']));movie=b''
    for k,b in boxes(base[b'moov']):
        if k==b'trak' and dict(boxes(b))[b'mdia'].find(b'vide')>=0:continue
        if k==b'mvhd':b=bytearray(b);b[16:20]=word(600)
        movie+=atom(k,b)
    files['audio-only.mp4']=atom(b'ftyp',base[b'ftyp'])+atom(b'mdat',base[b'mdat'])+atom(b'moov',movie)
    payload=bytearray(base[b'mdat']);start=len(payload)-19200
    for i in range(2):
        samples=[((j*37+i*137)%2048-1024)for j in range(4800)];raw=struct.pack('<4800h',*samples);payload[start+i*9600:start+(i+1)*9600]=raw
        files[f'alternate-track-{i}.f32le']=b''.join(struct.pack('<f',v/32768)for v in samples)
    files['alternate-audio-only.mp4']=atom(b'ftyp',base[b'ftyp'])+atom(b'mdat',payload)+atom(b'moov',movie)
    files['manifest.json']=(json.dumps({'provenance':'Authored VP9/dual PCM MP4 and600ms audio-only variant,two distinct scalar PCM versions for atomic pathname replacement. Exact YUV/F32 oracles,no private media,encoder,FFmpeg or network. Tests rename owned temporary copies;fixture bytes never change.','audio_ms':600,'video_ms':1500,'sha256':{n:hashlib.sha256(b).hexdigest()for n,b in files.items()}},indent=2)+'\n').encode()
    return files
if __name__=='__main__':
    OUT.mkdir(parents=True,exist_ok=True)
    for n,b in generate().items():(OUT/n).write_bytes(b)
