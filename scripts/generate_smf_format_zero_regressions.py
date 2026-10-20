"""Author SMF format0 multi-track admission versus valid topology controls."""
import json,hashlib
from pathlib import Path
from generate_smf_independent_tracks_regressions import generate as tracks
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/smf-format-zero'
def generate():
    prior=tracks();bad=bytearray(prior['simultaneous.mid']);bad[8:10]=b'\0\0'
    files={'video.mkv':prior['video.mkv'],'multi-zero.mid':bytes(bad),'format-one.mid':prior['simultaneous.mid'],'single-zero.mid':prior['single0.mid']}
    files['manifest.json']=(json.dumps(dict(provenance='Authored two-track simultaneous100/200ms notes withformat0header versus validformat1andone-trackformat0controls. Format0ntrksmust equal1. Correctrefusal,not acceptance ofinvalidtopology. Native VP9/PCMcompanion,no private data/encoder/network.',sha256={n:hashlib.sha256(v).hexdigest() for n,v in files.items()}),indent=2)+'\n').encode();return files
if __name__=='__main__':
    OUT.mkdir(parents=True,exist_ok=True)
    for n,b in generate().items():(OUT/n).write_bytes(b)
