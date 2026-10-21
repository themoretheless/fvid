#!/usr/bin/env python3
"""Author caption-paint inputs; reuse only the owned indexed VP9 clip."""
import hashlib
import json
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT/'tests/fixtures/playback-errors/subtitle-painting'

def generate():
    texts = {
        'two-lines': 'Первая α\nстрока',
        'wrapped': ('A deliberately long caption that wraps to several rows without leaving the video frame. '*3).strip(),
        'outline': 'Outlined caption',
    }
    files = {name+'.srt': ('1\n00:00:01,000 --> 00:00:09,000\n'+text+'\n\n').encode() for name,text in texts.items()}
    files['video.webm'] = (ROOT/'tests/fixtures/playback-errors/vp9-long-gop-seek.webm').read_bytes()
    files['manifest.json'] = (json.dumps({'provenance':'Authored SRT words,newlines and long wrapped text. Indexed video is the existing owned testsrc VP9 fixture,not private media. Regeneration copies that checked-in authored fixture;no encoder or network is needed. Native visual reproduction: fvid play video.webm --sub-file two-lines.srt --start-time 1.5 --start-paused --no-audio.','cases':list(texts),'sha256':{n:hashlib.sha256(b).hexdigest() for n,b in files.items()}},ensure_ascii=False,indent=2)+'\n').encode()
    return files

if __name__ == '__main__':
    OUT.mkdir(parents=True,exist_ok=True)
    for name,data in generate().items():
        (OUT/name).write_bytes(data)
