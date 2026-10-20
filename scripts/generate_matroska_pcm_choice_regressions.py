"""Authored tracks with unsupported default PCM and a valid alternative."""
import hashlib,json
from pathlib import Path
from generate_matroska_enabled_track_regressions import audiovisual
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/matroska-pcm-choice'
def generate():
    files={}
    for name,width,codec in [('valid-first',16,b'A_PCM/INT/LIT'),('integer-default',272,b'A_PCM/INT/LIT'),('float-default',288,b'A_PCM/FLOAT/IEEE'),('opus-missing-setup',16,b'A_OPUS'),('aac-missing-setup',16,b'A_AAC'),('vorbis-missing-setup',16,b'A_VORBIS')]:
        media,pcm=audiovisual(all_usable=True,defaults=(b'\1',b'\0'),audio_widths=(width,16),audio_codecs=(codec,b'A_PCM/INT/LIT'))
        files[name+'.mkv']=media
        files[name+'-expected.f32le']=bytes(len(pcm)) if name=='valid-first' else pcm
    files['manifest.json']=(json.dumps({'provenance':'Authored60ms VP9 plus default PCM and a valid nondefault alternative. Unsupported PCM widths and missing Opus/AAC/Vorbis setup must not block automatic selection of16bit PCM. No encoder/private media/network.','sha256':{k:hashlib.sha256(v).hexdigest() for k,v in files.items()}},indent=2)+'\n').encode()
    return files
if __name__=='__main__':
    OUT.mkdir(parents=True,exist_ok=True)
    for name,data in generate().items():(OUT/name).write_bytes(data)
