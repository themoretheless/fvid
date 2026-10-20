"""Reference authored ADTS LC/SBR/PS cases for exact output-sample seek."""
import json,hashlib
from pathlib import Path
from generate_audio_reader_eof_regressions import generate as pcm
ROOT=Path(__file__).resolve().parents[1]
OUT=ROOT/'tests/fixtures/playback-errors/aac-exact-seek'
def generate():
    files={'video.mkv':pcm()['video.mkv']};cases=[]
    for name,source in [('lc-stereo','audio/aac-stereo.aac'),('lc-mono44','audio/aac-mono-44k.aac'),('sbr','playback-errors/adts-multi-sbr-74-3-crc-synthetic.aac'),('ps','playback-errors/adts-implicit-ps-3-crc-synthetic.aac')]:
        data=(ROOT/'tests/fixtures'/source).read_bytes();cases.append(dict(name=name,source=source,sha256=hashlib.sha256(data).hexdigest()))
    for name,source in [('sbr-warm','playback-errors/adts-multi-sbr-74-3-crc-synthetic.aac'),('ps-warm','playback-errors/adts-implicit-ps-3-crc-synthetic.aac')]:
        data=(ROOT/'tests/fixtures'/source).read_bytes()*12;files[name+'.aac']=data;cases.append(dict(name=name,file=name+'.aac',sha256=hashlib.sha256(data).hexdigest()))
    files['manifest.json']=(json.dumps(dict(provenance='Existing authored ADTS LC/SBR/PS sources read in place. Odd decoded output-sample targets verify precision across core/output-rate ratios and warmed checkpoints. Full continuous native PCM is the seek consistency control. Separate short native VP9/PCM video. No private input/encoder/network.',cases=cases),indent=2)+'\n').encode();return files
if __name__=='__main__':
    OUT.mkdir(parents=True,exist_ok=True)
    for n,b in generate().items():(OUT/n).write_bytes(b)
