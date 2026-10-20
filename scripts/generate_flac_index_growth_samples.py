"""Author deterministic FLAC index-growth measurement inputs without an encoder."""
import json,hashlib
from pathlib import Path
from generate_flac_coded_numbers_regressions import frame,stream
from generate_audio_reader_eof_regressions import generate as pcm
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/flac-index-growth'
def generate():
    files={'video.mkv':pcm()['video.mkv']};cases=[]
    for count in [129,1024,8192]:
        file=f'frames-{count}.flac';files[file]=stream([frame(i,64,False,[4096,-4096,8192][i%3]) for i in range(count)],[64]*count);cases.append(dict(file=file,frames=count,samples=count*64))
    files['manifest.json']=(json.dumps(dict(provenance='Authored constant16bitFLAC with129/1024/8192short64sampleframes.129frame input is187ms;larger inputs are explicit index measurements,notnew playback-failure claims. Deterministic public PCM andnativeVP9/PCMcompanion,no private data/encoder/network.',cases=cases,sha256={n:hashlib.sha256(v).hexdigest() for n,v in files.items()}),indent=2)+'\n').encode();return files
if __name__=='__main__':
    OUT.mkdir(parents=True,exist_ok=True)
    for n,b in generate().items():(OUT/n).write_bytes(b)
