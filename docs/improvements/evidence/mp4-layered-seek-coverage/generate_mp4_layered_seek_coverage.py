#!/usr/bin/env python3
"""Owned independent HEVC layers with explicit presentation/decode ordering."""
from pathlib import Path
import hashlib,json
from generate_hevc_inband_layered_dispatch import inventory
from generate_hevc_layered_reconstruction import configuration
from hevc_fixture_mp4 import mux
ROOT=Path(__file__).resolve().parents[1]
OUT=ROOT/'tests/fixtures/playback-errors/mp4-layered-seek-coverage'
def generate():
    cfg,packets,size=inventory(ROOT/'tests/fixtures/playback-errors/hevc-layered-reconstruct-pcm-independent.mp4')
    params=configuration(cfg);prefix=[n for k in [32,33,34]for n in params[k]]
    groups=[packets[0] for _ in range(4)]
    stream=b''.join(b'\0\0\0\1'+n for n in prefix+[n for group in groups for n in group])
    OUT.mkdir(parents=True,exist_ok=True);manifest={}
    for name,order in [('ordered',[0,1,2,3]),('reordered',[0,2,1,3])]:
        data=mux(stream,8+cfg[17]%8,*size,chroma_format=cfg[16]%4,chroma_depth=8+cfg[18]%8,access_units=groups,multilayer=True,presentation_order=order)
        (OUT/(name+'.mp4')).write_bytes(data);manifest[name+'.mp4']={'sha256':hashlib.sha256(data).hexdigest(),'presentation_order':order}
    (OUT/'manifest.json').write_text(json.dumps(manifest,indent=2)+'\n')
if __name__=='__main__':generate()
