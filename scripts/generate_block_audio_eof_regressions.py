"""Record synthetic FLAC/AC3 references and author a short native video control."""
import hashlib,json
from pathlib import Path
from generate_audio_reader_eof_regressions import generate as pcm
ROOT=Path(__file__).resolve().parents[1]
OUT=ROOT/'tests/fixtures/playback-errors/block-audio-eof'
def generate():
    files={'video.mkv':pcm()['video.mkv']};cases=[]
    for kind,source in [('flac','playback-errors/flac-native/16-0-verbatim-0.flac'),('ac3','audio/ac3-stereo.ac3'),('mp2','mp3/layer-ii.mp2')]:
        data=(ROOT/'tests/fixtures'/source).read_bytes();cases.append(dict(kind=kind,source=source,sha256=hashlib.sha256(data).hexdigest()))
    files['manifest.json']=(json.dumps(dict(provenance='Existing authored synthetic audio references read in place; separate20ms native VP9/PCM video control. This regression targets raw elementary audio EOF seek, not Matroska demux or physical output.',cases=cases),indent=2)+'\n').encode();return files
if __name__=='__main__':
    OUT.mkdir(parents=True,exist_ok=True)
    for n,b in generate().items():(OUT/n).write_bytes(b)
