#!/usr/bin/env python3
"""Author canonical SAMI Sync clocks and an independent SubRip oracle."""
import json
from pathlib import Path

OUT = Path(__file__).resolve().parents[1] / 'tests/fixtures/playback-errors/sami-sync'

def generate():
    cases = {
        'invalid-clock': '<SYNC Start=1000><P Class=EN>Первая<SYNC Start=18446744073709551616><P Class=EN>Poison<SYNC Start=3000><P Class=EN>&nbsp;',
        'clear': '<SYNC Start=1000><P Class=EN>Первая<SYNC Start=2000><P Class=EN>&nbsp;',
        'replace': '<SYNC Start=1000><P Class=EN>Первая<SYNC Start=2000><P Class=EN>Вторая<SYNC Start=3000><P Class=EN>&nbsp;',
        'quoted': '<sYnC Start = "1000"><p Class=EN>Первая<Sync Start=\'2000\'><P Class=EN>&nbsp;',
        'language-gap': '<SYNC Start=1000><P Class=EN>Первая<P Class=FR>Un<SYNC Start=2000><P Class=FR>Deux<SYNC Start=3000><P Class=EN>&nbsp;<SYNC Start=4000><P Class=EN>Вторая<SYNC Start=5000><P Class=EN>&nbsp;',
        'lines': '<SYNC Start=1000><P Class=EN>Первая<br>строка<SYNC Start=2000><P Class=EN>&nbsp;',
    }
    expected = {
        'invalid-clock': [(1, 3, 'Первая')],
        'clear': [(1, 2, 'Первая')],
        'replace': [(1, 2, 'Первая'), (2, 3, 'Вторая')],
        'quoted': [(1, 2, 'Первая')],
        'language-gap': [(1, 3, 'Первая'), (4, 5, 'Вторая')],
        'lines': [(1, 2, 'Первая\nстрока')],
    }
    files = {}
    for name, body in cases.items():
        files[name + '.smi'] = ('<SAMI><HEAD></HEAD><BODY>' + body + '</BODY></SAMI>\n').encode()
        files[name + '.srt'] = ''.join(f'{i}\n00:00:{a:02},000 --> 00:00:{b:02},000\n{t}\n\n' for i, (a,b,t) in enumerate(expected[name], 1)).encode()
    files['video.y4m'] = b'YUV4MPEG2 W2 H2 F1:1 Ip A1:1 C420jpeg\n' + b''.join(b'FRAME\n' + bytes([32+n*32]*4+[128,128]) for n in range(5))
    files['manifest.json'] = (json.dumps({'provenance': 'Authored UTF8 captions, literal SubRip clocks and five raw Y4M frames; no private media or external tools.', 'cases': list(cases)}, indent=2)+'\n').encode()
    return files

if __name__ == '__main__':
    OUT.mkdir(parents=True, exist_ok=True)
    for name, data in generate().items():
        (OUT/name).write_bytes(data)
