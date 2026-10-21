#!/usr/bin/env python3
"""Authored SAMI attribute boundaries, quoted values and raw Y4M companion."""
import json,hashlib
from pathlib import Path
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/sami-attributes'
def generate():
    def doc(body):return ('<SAMI><BODY>'+body+'</BODY></SAMI>\n').encode()
    def p(attrs,text='Строка α'):return '<P '+attrs+'>'+text+'</P>'
    normal='Begin=1000 End=2000 Class=EN'
    standard=p(normal)
    cases=[
      ('quoted-begin-single',p('title=\'begin="0"\' '+normal),standard,True),
      ('quoted-begin-double',p('title="begin=\'0\'" '+normal),standard,True),
      ('hyphenated-name',p('data-begin=0 '+normal),standard,True),
      ('underscored-name',p('data_begin=0 '+normal),standard,True),
      ('quoted-end',p('title=\'end="1100"\' '+normal),standard,True),
      ('quoted-class',p('title=\'class="fake"\' '+normal)+p('Begin=3000 End=4000 Class=EN','Вторая'),standard+p('Begin=3000 End=4000 Class=EN','Вторая'),True),
      ('untimed-title-only',p('title=\'begin="0"\' End=2000'),p('End=2000'),True),
      ('opposite-quote-in-class',p('Begin=1000 End=2000 Class="en\'gb"')+p('Begin=2000 End=3000 Class=en','Other')+p('Begin=3000 End=4000 Class="en\'gb"','Вторая'),p('Begin=1000 End=2000 Class=main')+p('Begin=2000 End=3000 Class=other','Other')+p('Begin=3000 End=4000 Class=main','Вторая'),True),
      ('real-before-title',p('Begin=1000 title=\'begin="0"\' End=2000 Class=EN'),standard,False),
      ('quoted-whitespace',p('Begin = "1000" End = \'2000\' Class = "EN"'),standard,False),
      ('unrelated-title-words',p('title="begin end class" '+normal),standard,False),
      ('longer-name-control',p('beginners=0 endings=0 '+normal),standard,False)]
    files={};records=[]
    for name,actual,control,bad in cases:
        files[name+'.smi']=doc(actual);files[name+'-control.smi']=doc(control);records.append(dict(name=name,file=name+'.smi',control=name+'-control.smi',before_failure=bad,kind='skip-untimed' if name=='untimed-title-only' else 'caption-acceptance'))
    video=b'YUV4MPEG2 W2 H2 F1:1 Ip A1:1 C420jpeg\n'
    for n in range(5):video+=b'FRAME\n'+bytes([32+n*32]*4+[128,128])
    files['video.y4m']=video
    files['manifest.json']=(json.dumps({'provenance':'Authored SAMI quoted attribute-like strings,whole-name boundaries and matching-quote class values versus plain-attribute controls. Explicit1..2s and3..4s caption clocks,UTF8text,and five raw2x2Y4Mframes. Untimed input is a correct-skip test,not caption acceptance. No external codec,network or private media.','cases':records,'sha256':{n:hashlib.sha256(data).hexdigest() for n,data in files.items()}},ensure_ascii=False,indent=2)+'\n').encode();return files
if __name__=='__main__':
    OUT.mkdir(parents=True,exist_ok=True)
    for n,data in generate().items():(OUT/n).write_bytes(data)
