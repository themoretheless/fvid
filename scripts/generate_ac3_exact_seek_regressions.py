"""Record existing short authored tone/noise AC3 references and short video control."""
import json,hashlib
from pathlib import Path
from generate_audio_reader_eof_regressions import generate as pcm
ROOT=Path(__file__).resolve().parents[1]
OUT=ROOT/'tests/fixtures/playback-errors/ac3-exact-seek'
def generate():
    files={'video.mkv':pcm()['video.mkv']};cases=[]
    for name,source,rate,channels in [('stereo','audio/ac3-stereo.ac3',48000,2),('coupled','audio/ac3-coupled.ac3',48000,2),('mono32','audio/ac3-mono-32k.ac3',32000,1)]:
        data=(ROOT/'tests/fixtures'/source).read_bytes();cases.append(dict(name=name,source=source,rate=rate,channels=channels,sha256=hashlib.sha256(data).hexdigest()))
    files['manifest.json']=(json.dumps(dict(provenance='Existing short authored native AC3 stereo tone/coupled pink-noise/32k mono references read in place. Sample-exact seek is compared with full continuous native decoding; no reference-quality codec claim. Short authored VP9/PCM companion. No private input/encoder/network.',cases=cases),indent=2)+'\n').encode();return files
if __name__=='__main__':
    OUT.mkdir(parents=True,exist_ok=True)
    for n,b in generate().items():(OUT/n).write_bytes(b)
