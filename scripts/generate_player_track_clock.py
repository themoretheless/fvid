#!/usr/bin/env python3
"""Authored VP9 plus two scalar PCM MP4 tracks for exact track-switch clocks."""
from pathlib import Path
import hashlib,json,struct
from generate_player_rate_clock import generate as video
from generate_mp4_enabled_track_regressions import atom,word,boxes,canonical_movie
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/player-track-clock'
def generate():
    seed=video();files={n:b for n,b in seed.items()if n not in ('movie.mp4','manifest.json')};base=dict(boxes(seed['movie.mp4']));ftyp=atom(b'ftyp',base[b'ftyp']);movie=dict(boxes(base[b'moov']));payload=base[b'mdat'];tracks=atom(b'trak',movie[b'trak'])
    for index in range(2):
        samples=[((i*17+index*79)%1024-512)*(1 if index==0 else -1)for i in range(4800)];raw=struct.pack('<4800h',*samples);offset=len(ftyp)+8+len(payload);payload+=raw
        entry=bytearray(28);entry[6:8]=b'\0\1';entry[16:20]=struct.pack('>HH',1,16);entry[24:28]=word(8000<<16)
        tables=atom(b'stsd',word(0,1)+atom(b'sowt',entry))+atom(b'stts',word(0,1,4800,1))+atom(b'stsz',word(0,2,4800))+atom(b'stsc',word(0,1,1,4800,1))+atom(b'stco',word(0,1,offset))
        mdhd=bytearray(24);mdhd[12:20]=word(8000,4800);tkhd=bytearray(84);tkhd[:4]=word(3);tkhd[12:16]=word(index+2);tkhd[20:24]=word(600);tkhd[40:44]=tkhd[56:60]=word(65536);tkhd[72:76]=word(1<<30)
        tracks+=atom(b'trak',atom(b'tkhd',tkhd)+atom(b'mdia',atom(b'mdhd',mdhd)+atom(b'hdlr',word(0,0)+b'soun'+bytes(12))+atom(b'minf',atom(b'dinf',atom(b'dref',word(0,1)+atom(b'url ',word(1))))+atom(b'stbl',tables)))+atom(b'udta',atom(b'name',f'authored PCM {index}'.encode())))
        files[f'track-{index}.f32le']=b''.join(struct.pack('<f',v/32768)for v in samples)
    files['movie.mp4']=ftyp+atom(b'mdat',payload)+atom(b'moov',atom(b'mvhd',canonical_movie(movie[b'mvhd'],4))+tracks)
    files['manifest.json']=(json.dumps({'provenance':'Authored1.5s VP9 five16x16 keyframes(300ms each),two distinct600ms mono8kHz PCM tracks and exact scalar YUV/PCM oracles. No private media,FFmpeg,encoder or network.','video_ms':1500,'audio_ms':600,'switch_ms':[75,175],'sha256':{n:hashlib.sha256(b).hexdigest()for n,b in files.items()}},indent=2)+'\n').encode()
    return files
if __name__=='__main__':
    OUT.mkdir(parents=True,exist_ok=True)
    for n,b in generate().items():(OUT/n).write_bytes(b)
