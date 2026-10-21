#!/usr/bin/env python3
"""Author body-looking tokens inside values/comments, separate from clocks."""
from pathlib import Path
import json

OUT = Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/sami-body-boundaries'

def generate():
    paragraph = '<P Begin=1000 End=2000>Строка α</P>'
    cases = {
        'quoted-body-close': "<P title='</body>' Begin=1000 End=2000>Строка α</P>",
        'comment-body-close': '<!-- </body> -->'+paragraph,
        'title-control': "<P title='body' Begin=1000 End=2000>Строка α</P>",
        'comment-control': '<!-- body -->'+paragraph,
    }
    files = {name+'.smi': ('<SAMI><BODY>'+body+'</BODY></SAMI>\n').encode() for name,body in cases.items()}
    for name,header in [('title-raw-close','<TITLE>marker </body> text</TITLE>'), ('style-raw-close','<STYLE>p {color:white;} /* </body> */</STYLE>')]:
        cases[name] = header
        files[name+'.smi'] = ('<SAMI><HEAD>'+header+'</HEAD><BODY>'+paragraph+'</BODY></SAMI>\n').encode()
    files['control.srt'] = '1\n00:00:01,000 --> 00:00:02,000\nСтрока α\n\n'.encode()
    files['video.y4m'] = b'YUV4MPEG2 W2 H2 F1:1 Ip A1:1 C420jpeg\n'+b''.join(b'FRAME\n'+bytes([32+n*32]*4+[128,128]) for n in range(5))
    files['manifest.json'] = (json.dumps({'provenance':'Authored quoted values,HTML comments and Title/Style raw header text; explicit legacy paragraph clocks isolate body boundary discovery from canonical Sync timing. Independent SRT and five raw Y4M frames. No external tools or private media.', 'cases':list(cases)},indent=2)+'\n').encode()
    return files

if __name__ == '__main__':
    OUT.mkdir(parents=True, exist_ok=True)
    for name,data in generate().items():
        (OUT/name).write_bytes(data)
