"""Authored preferred movie PCM geometry and valid alternate witnesses."""
import hashlib,json
from pathlib import Path
from generate_mp4_enabled_track_regressions import audiovisual
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/mp4-pcm-choice'
def generate():
    files={}
    for name,width,flags in [('valid-movie',16,(3,1)),('unsupported-movie',272,(3,1)),('both-movie',272,(3,3))]:
        media,pcm=audiovisual(audio_widths=(width,16),audio_flags=flags)
        files[name+'.mp4']=media
        files[name+'-expected.f32le']=bytes(len(pcm)) if name=='valid-movie' else pcm
    files['manifest.json']=(json.dumps({'provenance':'Authored60ms VP9/two PCM MP4 tracks. Unsupported272bit preferred movie track must not block valid16bit PCM. No encoder/private media/network.','sha256':{k:hashlib.sha256(v).hexdigest() for k,v in files.items()}},indent=2)+'\n').encode()
    return files
if __name__=='__main__':
    OUT.mkdir(parents=True,exist_ok=True)
    for name,data in generate().items():(OUT/name).write_bytes(data)
