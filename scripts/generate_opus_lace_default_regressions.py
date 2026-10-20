"""Authored variable Opus laces with an optional default packet duration."""
import hashlib,json
from pathlib import Path
from generate_matroska_opus_endpoint_regressions import clip
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/opus-lace-default'
def generate():
    files={}
    for mode in ['xiph','ebml']:
        for label,default in [('control',0),('default',20_000_000),('default-short',1_000_000),('default-long',120_000_000)]:files[mode+'-'+label+'.mkv']=clip(mode,0,default_duration=default)[0]
        files[mode+'-unsupported-head.mkv']=clip(mode,0,default_duration=20_000_000)[0].replace(b'OpusHead\x01',b'OpusHead\x00')
    files['manifest.json']=(json.dumps({'provenance':'Authored variable20/40/10/20ms Opus DTX packets,90ms BlockDuration, optional20ms Track DefaultDuration and five VP9 frames. No private media, encoder or network.','starts_ns':[0,20_000_000,60_000_000,70_000_000],'durations_ns':[20_000_000,40_000_000,10_000_000,20_000_000],'sha256':{n:hashlib.sha256(b).hexdigest() for n,b in files.items()}},indent=2)+'\n').encode()
    return files
if __name__=='__main__':
    OUT.mkdir(parents=True,exist_ok=True)
    for n,b in generate().items():(OUT/n).write_bytes(b)
