"""Derive a64ms two-frame native tone to compare grouped and packeted presentation."""
import json,hashlib
from pathlib import Path
from generate_audio_reader_eof_regressions import generate as pcm
ROOT=Path(__file__).resolve().parents[1]
OUT=ROOT/'tests/fixtures/playback-errors/ac3-presentation-delay'
def generate():
    source=(ROOT/'tests/fixtures/audio/ac3-stereo.ac3').read_bytes();width=len(source)//8;files={'video.mkv':pcm()['video.mkv'],'stream.ac3':source[:2*width]}
    files['manifest.json']=(json.dumps(dict(provenance='First two complete frames of existing authored synthetic AC3 tone, unchanged CRC-protected packets. Native grouped PCM is compared with per-frame reader presentation and worker output after intrinsic256sample leading trim. Short native VP9/PCM video. No private input/encoder/network.',source_sha256=hashlib.sha256(source).hexdigest(),sha256={n:hashlib.sha256(v).hexdigest() for n,v in files.items()}),indent=2)+'\n').encode();return files
if __name__=='__main__':
    OUT.mkdir(parents=True,exist_ok=True)
    for n,b in generate().items():(OUT/n).write_bytes(b)
