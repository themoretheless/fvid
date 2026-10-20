"""Authored120ms video with an80ms gap between PCM packets."""
import hashlib,json,struct
from pathlib import Path
from generate_audio_ring_capacity_regressions import clip
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/audio-packet-timeline'
def generate():
    media,pcm=clip(160,8000,6,packets=2,packet_times=[0,100])
    files={'gap.mkv':media,'expected-packet.f32le':pcm,'expected-output.f32le':pcm+b'\0'*(640*4)+pcm}
    for name,times,video_frames,expected in [('overlap',[0,10],2,pcm+pcm[80*4:]),('duplicate',[0,0],1,pcm),('leading',[100],6,b'\0'*(800*4)+pcm)]:
        video,_=clip(160,8000,video_frames,packets=len(times),packet_times=times)
        files[name+'.mkv']=video
        files[name+'-expected.f32le']=expected
    files['manifest.json']=(json.dumps({'provenance':'Authored120ms neutral VP9 with two20ms8kHz PCM packets at0ms and100ms. Expected80ms silence preserves the declared gap. No encoder/private media/network.','sha256':{k:hashlib.sha256(v).hexdigest() for k,v in files.items()}},indent=2)+'\n').encode()
    return files
if __name__=='__main__':
    OUT.mkdir(parents=True,exist_ok=True)
    for name,data in generate().items():(OUT/name).write_bytes(data)
