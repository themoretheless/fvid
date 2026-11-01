#!/usr/bin/env python3
"""Author exact PCM MP4 navigation clocks and a valid video companion;offline."""
from pathlib import Path
import hashlib,json,struct
from generate_player_rate_clock import generate as video
from generate_mp4_enabled_track_regressions import atom,word,boxes,canonical_movie
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/player-audio-seek-intent'
def generate():
    files={n:b for n,b in video().items() if n!='manifest.json'}
    samples=[i*17%1024-512 for i in range(1600)];raw=struct.pack('<1600h',*samples)
    base=dict(boxes(files['movie.mp4']));ftyp=atom(b'ftyp',base[b'ftyp'])
    entry=bytearray(28);entry[6:8]=b'\0\1';entry[16:20]=struct.pack('>HH',1,16);entry[24:28]=word(8000<<16)
    tables=atom(b'stsd',word(0,1)+atom(b'sowt',entry))+atom(b'stts',word(0,1,1600,1))+atom(b'stsz',word(0,2,1600))+atom(b'stsc',word(0,1,1,1600,1))+atom(b'stco',word(0,1,len(ftyp)+8))
    mdhd=bytearray(24);mdhd[12:20]=word(8000,1600)
    tkhd=bytearray(84);tkhd[:4]=word(3);tkhd[12:16]=word(1);tkhd[20:24]=word(200);tkhd[40:44]=tkhd[56:60]=word(65536);tkhd[72:76]=word(1<<30)
    mvhd=canonical_movie(dict(boxes(base[b'moov']))[b'mvhd'],2);mvhd[12:20]=word(1000,200)
    track=atom(b'trak',atom(b'tkhd',tkhd)+atom(b'mdia',atom(b'mdhd',mdhd)+atom(b'hdlr',word(0,0)+b'soun'+bytes(12))+atom(b'minf',atom(b'dinf',atom(b'dref',word(0,1)+atom(b'url ',word(1))))+atom(b'stbl',tables))))
    files['sound.mp4']=ftyp+atom(b'mdat',raw)+atom(b'moov',atom(b'mvhd',mvhd)+track)
    files['sound.f32le']=b''.join(struct.pack('<f',v/32768)for v in samples)
    files['manifest.json']=(json.dumps({'provenance':'Authored200ms mono8kHz PCM MP4 and1.5s five-frame16x16 VP9 video companion. Exact independent scalar PCM/YUV;actual backend flush can be delayed without private media,FFmpeg,network or codec parameter copying.','audio_ms':200,'video_ms':1500,'sha256':{n:hashlib.sha256(b).hexdigest()for n,b in files.items()}},indent=2)+'\n').encode()
    return files
if __name__=='__main__':
    OUT.mkdir(parents=True,exist_ok=True)
    for n,b in generate().items():(OUT/n).write_bytes(b)
