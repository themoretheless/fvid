#!/usr/bin/env python3
"""Isolate case-insensitive SAMI line breaks from Sync support."""
from pathlib import Path
import json

OUT = Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/sami-break-case'

def generate():
    files = {}
    for name, tag in [('upper', '<BR>'), ('mixed', '<bR/>'), ('attributes', '<Br class=line>'), ('lower-control', '<br>')]:
        files[name+'.smi'] = ('<SAMI><BODY><P Begin=1000 End=2000>Первая'+tag+'строка</P></BODY></SAMI>\n').encode()
    files['control.srt'] = '1\n00:00:01,000 --> 00:00:02,000\nПервая\nстрока\n\n'.encode()
    files['video.y4m'] = b'YUV4MPEG2 W2 H2 F1:1 Ip A1:1 C420jpeg\n'+b''.join(b'FRAME\n'+bytes([32+n*32]*4+[128,128]) for n in range(5))
    files['manifest.json'] = (json.dumps({'provenance':'Authored caption text and raw Y4M companion. Explicit paragraph clocks isolate break case handling from unsupported canonical Sync timing. No private media or codec parameters.', 'cases':['upper','mixed','attributes','lower-control']}, indent=2)+'\n').encode()
    return files

if __name__ == '__main__':
    OUT.mkdir(parents=True, exist_ok=True)
    for name, data in generate().items():
        (OUT/name).write_bytes(data)
