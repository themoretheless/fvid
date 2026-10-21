#!/usr/bin/env python3
"""Author independently playable subtitles whose .sami extension was missed."""
import json
from pathlib import Path

OUT = Path(__file__).resolve().parents[1] / 'tests/fixtures/playback-errors/sami-sidecar-extension'

def generate():
    files = {}
    caption = '<SAMI><BODY><P Begin=1000 End=2000>Строка α</P></BODY></SAMI>\n'.encode()
    video = b'YUV4MPEG2 W2 H2 F1:1 Ip A1:1 C420jpeg\n' + b''.join(b'FRAME\n' + bytes([32+n*32]*4+[128,128]) for n in range(5))
    cases = [('lower','video.sami'), ('upper','video.SAMI'), ('loose','video.en.sami'), ('smi-control','video.smi')]
    for name, subtitle in cases:
        files[name+'/video.y4m'] = video
        files[name+'/'+subtitle] = caption
        files[name+'/unrelated.sami'] = caption
    files['manifest.json'] = (json.dumps({'provenance': 'Authored plain paragraph clocks isolate extension discovery from canonical Sync support. Five raw Y4M frames per directory; no private media, network or external tools.', 'cases':[{'name':name, 'subtitle':subtitle} for name,subtitle in cases]}, indent=2)+'\n').encode()
    return files

if __name__ == '__main__':
    for name, data in generate().items():
        path = OUT/name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(data)
