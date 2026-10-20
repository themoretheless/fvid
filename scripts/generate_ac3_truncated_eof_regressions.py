"""Derive short AC3 truncations from existing authored tone, no encoder."""
import json,hashlib
from pathlib import Path
from generate_audio_reader_eof_regressions import generate as pcm
ROOT=Path(__file__).resolve().parents[1]
OUT=ROOT/'tests/fixtures/playback-errors/ac3-truncated-eof'
def generate():
    source=(ROOT/'tests/fixtures/audio/ac3-stereo.ac3').read_bytes();width=len(source)//8
    assert len(source)%8==0 and all(source[i*width:i*width+2]==b'\x0b\x77' for i in range(8))
    full=source*2;prefix=full[:-width]
    files={'video.mkv':pcm()['video.mkv'],'full.ac3':full,'prefix.ac3':prefix,'padded.ac3':prefix+bytes(16),'truncated-one.ac3':full[:-1],'truncated-half.ac3':full[:-width//2]}
    files['manifest.json']=(json.dumps(dict(provenance='Existing authored synthetic stereo AC3 tone repeated into16frames, then final frame truncated by1byte or half.15frame genuine-prefix and nonaudio zero-tail controls. Native prefixes remain usable; known incomplete last frame must fail EOF. Short authored VP9/PCM companion. No private source/network/encoder.',source_sha256=hashlib.sha256(source).hexdigest(),frame_bytes=width,sha256={n:hashlib.sha256(v).hexdigest() for n,v in files.items()}),indent=2)+'\n').encode();return files
if __name__=='__main__':
    OUT.mkdir(parents=True,exist_ok=True)
    for n,b in generate().items():(OUT/n).write_bytes(b)
