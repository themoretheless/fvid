#!/usr/bin/env python3
"""Short authored block-boundary, equal and signed PTS seek controls."""
from pathlib import Path
import hashlib,json
from generate_mp4_reordered_seek_workload import build
from generate_mp4_seek_io_regressions import generate as seed_files
from generate_vp9_chroma import mp4,atom,word
from generate_mp4_fragment_description_regressions import atoms
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/mp4-reordered-seek'
def generate():
    seed=seed_files();files={f'frame-{i}.yuv16':seed[f'frame-{i}.yuv16']for i in range(3)};cases=[]
    def replace(body,shifts):
        out=b''
        for k,b in atoms(body):
            if k in [b'moov',b'trak',b'mdia',b'minf',b'stbl']:b=replace(b,shifts)
            elif k==b'ctts':b=word(1<<24,len(shifts))+b''.join(word(1)+int(d).to_bytes(4,'big',signed=True)for d in shifts)
            out+=atom(k,b)
        return out
    for count in [3,33,96,99]:
        name=f'reordered-{count}.mp4';files[name]=build(count);cases.append({'file':name,'count':count,'pts':[i+[0,1,-1][i%3]for i in range(count)]})
    count=99
    name='ordered-99.mp4';files[name]=mp4([seed[f'frame-{i%3}.packet']for i in range(count)],8,'420',size=(16,16),sync_samples=list(range(1,count+1)));cases.append({'file':name,'count':count,'pts':list(range(count))})
    for label,shifts in [('equal',[i//3*3-i for i in range(count)]),('negative',[[0,1,-1][i%3]-2 for i in range(count)])]:
        name=label+'-99.mp4';files[name]=replace(build(count),shifts);cases.append({'file':name,'count':count,'pts':[i+d for i,d in enumerate(shifts)]})
    for label,pts in [('descending',list(reversed(range(count)))),('late-high',[0,count-1,*range(1,count-1)])]:
        name=label+'-99.mp4';files[name]=replace(build(count),[p-i for i,p in enumerate(pts)]);cases.append({'file':name,'count':count,'pts':pts})
    files['manifest.json']=(json.dumps({'provenance':'Existing authored three colored16x16 VP9 keyframes and exact plane oracles. All samples sync; 50Hz and <=99coded frames. Signed CTTS controls; no encoder,network or private media.','cases':cases,'sha256':{n:hashlib.sha256(b).hexdigest()for n,b in files.items()}},indent=2)+'\n').encode()
    return files
if __name__=='__main__':
    OUT.mkdir(parents=True,exist_ok=True)
    for n,b in generate().items():(OUT/n).write_bytes(b)
