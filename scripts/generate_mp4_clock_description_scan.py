"""Authored compact PCM with unused description catalogs; no encoder/network."""
from pathlib import Path
import json,hashlib
from generate_mp4_uniform_plan_scan import generate as compact
from generate_mp4_tfhd_defaults_regressions import atoms
from generate_vp9_chroma import atom,word
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/mp4-clock-description-scan'
def generate():
    original=compact();files={};cases=[]
    for count in [64,262144]:
        for unused in [0,1,32]:
            def rewrite(data):
                out=[]
                for k,b in atoms(data):
                    if k in [b'moov',b'trak',b'mdia',b'minf']:b=rewrite(b)
                    elif k==b'stbl' and b'sowt' in b:
                        tables=[]
                        for t,p in atoms(b):
                            if t==b'stsd':p=word(0,unused+1)+p[8:]*(unused+1)
                            tables.append(atom(t,p))
                        b=b''.join(tables)
                    out.append(atom(k,b))
                return b''.join(out)
            name=f'frames{count}-unused{unused}.mp4';files[name]=rewrite(original[f'frames-{count}.mp4']);cases.append(dict(file=name,frames=count,descriptions=unused+1,rate=48000))
    for mode in ['alias','unused','endian']:
        name=f'control-{mode}.mp4';files[name]=original[name];cases.append(dict(file=name,frames=64,descriptions=2,rate=48000))
    files['manifest.json']=(json.dumps(dict(provenance='Own compact constant PCM0.125,duplicate unused audio headers,and three neutral VP9 pictures. Original authored alias/unused/endian controls included. No private media,encoder or network.',cases=cases,sha256={n:hashlib.sha256(b).hexdigest()for n,b in files.items()}),indent=2)+'\n').encode();return files
if __name__=='__main__':
    OUT.mkdir(parents=True,exist_ok=True)
    for n,b in generate().items():(OUT/n).write_bytes(b)
