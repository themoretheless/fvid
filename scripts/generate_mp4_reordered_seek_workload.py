#!/usr/bin/env python3
"""Manual reordered-index workload; ordinary tests do not generate media."""
from pathlib import Path
import json,hashlib,sys
from generate_mp4_seek_io_regressions import generate
from generate_vp9_chroma import mp4,atom,word
from generate_mp4_fragment_description_regressions import atoms

def build(count):
    assert 3<=count<=300000 and count%3==0
    seed=generate();source=mp4([seed[f'frame-{i%3}.packet']for i in range(count)],8,'420',size=(16,16),sync_samples=list(range(1,count+1)))
    def walk(body):
        out=b''
        for k,b in atoms(body):
            if k in [b'moov',b'trak',b'mdia',b'minf',b'stbl']:b=walk(b)
            out+=atom(k,b)
            if k==b'stts':out+=atom(b'ctts',word(1<<24,count)+b''.join(word(1)+int([0,1,-1][i%3]).to_bytes(4,'big',signed=True)for i in range(count)))
        return out
    return walk(source)
if __name__=='__main__':
    p=Path(sys.argv[1]);count=int(sys.argv[2])if len(sys.argv)>2 else 60000
    data=build(count);p.write_bytes(data);p.with_suffix('.json').write_text(json.dumps({'provenance':'Authored three colored16x16 VP9 keyframes repeat with PTS permutation0,2,1;all packets sync. Manual CPU workload,not ordinary test input. No private media,encoder or network.','packets':count,'timescale':50,'sha256':hashlib.sha256(data).hexdigest(),'bytes':len(data)},indent=2)+'\n')
