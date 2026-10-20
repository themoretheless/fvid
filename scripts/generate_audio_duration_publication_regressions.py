#!/usr/bin/env python3
"""Author short VP9/Opus clips whose exact endpoint is learned at EOF."""
import json
from pathlib import Path
from generate_matroska_opus_endpoint_regressions import clip
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/audio-duration-publication'
def generate():
    files={}
    for name,padding in [('control',0),('tail',40_000_000),('empty',60_000_000)]:
        files[name+'.mkv']=clip('plain',padding)[0]
    files['manifest.json']=(json.dumps({'synthetic':True,'video':'authored VP9','audio':'authored Opus DTX','initial_ns':{'control':60_000_000,'tail':60_000_000,'empty':0},'end_ns':{'control':60_000_000,'tail':20_000_000,'empty':0}},indent=2)+'\n').encode()
    return files
if __name__=='__main__':
    OUT.mkdir(parents=True,exist_ok=True)
    for name,data in generate().items():(OUT/name).write_bytes(data)
