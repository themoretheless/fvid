"""Own changing PCM metadata and route-admission witnesses."""
from pathlib import Path
import hashlib,json
from generate_mp4_audio_reconfiguration import generate
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/native-mp4-initial-metadata'
def main():
    files={};cases=[]
    for profile,shapes,rate,channels,codec in [('changing-s16',None,48000,2,'pcm_s16le'),('changing-f32',[(b'fl32',32000,1,32),(b'sowt',48000,2,16),(b'in24',44100,2,24)],32000,1,'pcm_f32le'),('constant',[(b'fl32',32000,1,32)]*3,32000,1,'pcm_f32le')]:
        own=generate(pcm_shapes=shapes,pcm_only=True)
        frames=[]
        for i,(r,ch) in enumerate(([(48000,2),(32000,1),(44100,2)]*2) if shapes is None else [(shapes[j%3][1],shapes[j%3][2])for j in range(6)]):
            n=f'{profile}-{i}.f32le';files[n]=own[f'pcm-{i}.f32le'];frames.append(dict(rate=r,channels=ch,oracle=n))
        for source in ['pcm.mp4','pcm-fragmented.mp4','pcm-start-third.mp4','pcm-start-third-fragmented.mp4']:
            n=profile+'-'+source;files[n]=own[source];cases.append(dict(file=n,rate=rate,channels=channels,codec=codec,changing=profile!='constant',frames=frames))
    OUT.mkdir(parents=True,exist_ok=True)
    for n,b in files.items():(OUT/n).write_bytes(b)
    (OUT/'manifest.json').write_text(json.dumps({'provenance':'Own 120ms neutral16x16 VP9 and six authored nonzero PCM spans with coding/rate/channel/precision transitions or equivalent constant setups, in regular/fragmented form and reordered sample-description tables. No private media/parameters, external encoder, FFmpeg or network.','cases':cases,'sha256':{n:hashlib.sha256(b).hexdigest() for n,b in files.items()}},indent=2)+'\n')
if __name__=='__main__':main()
