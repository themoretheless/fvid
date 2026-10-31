#!/usr/bin/env python3
"""Authored chapter marks within one native video picture; offline."""
from pathlib import Path
import hashlib,json,struct
from generate_player_rate_clock import generate as source
from generate_mp4_fragment_description_regressions import atoms
from generate_vp9_chroma import atom,word
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/player-chapter-clock'
def generate():
    files={n:b for n,b in source().items() if n!='manifest.json'}
    starts=[0,100,200,600,1200]
    titles=['Opening','First interior','Second interior','Later','Final']
    chpl=word(1<<24,0)+bytes([len(starts)])
    for at,title in zip(starts,titles):
        text=title.encode();chpl+=struct.pack('>Q',at*10000)+bytes([len(text)])+text
    files['movie.mp4']=b''.join(atom(k,b+atom(b'udta',atom(b'chpl',chpl)) if k==b'moov' else b) for k,b in atoms(files['movie.mp4']))
    files['manifest.json']=(json.dumps({'provenance':'Five authored16x16 VP9 keyframes at300ms intervals;authored Nero chpl v1 chapter marks in10MHz clock. No private media,encoder,FFmpeg or network.','chapter_starts_ms':starts,'chapter_titles':titles,'sha256':{n:hashlib.sha256(b).hexdigest() for n,b in files.items()}},indent=2)+'\n').encode()
    return files
if __name__=='__main__':
    OUT.mkdir(parents=True,exist_ok=True)
    for n,b in generate().items():(OUT/n).write_bytes(b)
