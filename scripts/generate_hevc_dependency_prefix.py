#!/usr/bin/env python3
"""Original Annex F dependency-prefix metadata, not complete video streams."""
import json
from pathlib import Path
DEST=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors'
def packed(s):
    return int(s+'0'*((-len(s))%8),2).to_bytes((len(s)+7)//8,'big')
cases=[]
for name,ids,edges,split in [
    ('chain',[0,3,7,15],[(1,0),(2,1),(3,2)],False),
    ('diamond',[0,1,2,3],[(1,0),(2,0),(3,1),(3,2)],False),
    ('forest',[0,2,5,9],[(2,0),(2,1),(3,1)],False),
    ('split',[0,1,9,17],[(2,0),(3,2)],True),
    ('maximum',list(range(63)),[(i,i-1) for i in range(1,63)],False),
    ('duplicate',[0,3,3],[],False),
]:
    # Extra PTL: no sublayer flags, only general_level_idc.
    bits=f'{120:08b}'+str(int(split))
    bits+=''.join('1' if i in (1,2) else '0' for i in range(16))
    bits+='010' if split else '001001' # split 3+3; explicit 2+2 dimensions
    bits+='1'
    for i,id in enumerate(ids[1:],1):
        bits+=f'{id:06b}'
        if not split:bits+='00'+f'{i%4:02b}'
    views=len(set(id&7 for id in ids)) if split else 1
    bits+='0100'+''.join(f'{v:04b}' for v in range(views))
    bits+=''.join(str(int((i,j) in edges)) for i in range(1,len(ids)) for j in range(i))
    f='hevc-dependency-prefix-'+name+'.bin'
    (DEST/f).write_bytes(packed(bits))
    cases.append(dict(file=f,ids=ids,edges=edges,split=split,end=len(bits),error=name=='duplicate'))
(DEST/'hevc-dependency-prefix.json').write_text(json.dumps(cases,indent=2)+'\n')
