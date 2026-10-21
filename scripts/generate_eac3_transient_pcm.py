"""Independent scalar PCM oracle for E.2.7, not streaming decoder acceptance."""
import json,math,struct
from pathlib import Path
from generate_audio_ring_capacity_regressions import clip
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/eac3-transient-pcm'
def generate():
 video,raw=clip(frames=6144,bits=32,floating=True,video_frames=7)
 original=list(struct.unpack('<6144f',raw));files={'input.f32le':raw,'input.mkv':video};cases=[]
 for origin in [0,-1536]:
  for phase in [0,4,252]:
   for length in [0,1,128,255]:
    transient=origin+3072+phase
    previous_block=(transient//256-1)*256
    pn=transient-previous_block;total=pn+length+256
    source=transient-512-2*pn-origin;destination=transient-total-origin
    synthesis=original[source:source+512+pn];result=original.copy()
    for i in range(total):
     if i<256:
      w=(1-math.cos(math.pi*i/255))/2;value=original[destination+i]*(1-w)+synthesis[i]*w
     elif i>=total-128:
      w=(1-math.cos(math.pi*(i-total+128)/127))/2;value=original[destination+i]*w+synthesis[i]*(1-w)
     else:value=synthesis[i]
     result[destination+i]=value
    name=f'o{origin}-p{phase}-l{length}';files[name+'.f32le']=struct.pack('<6144f',*result)
    cases.append(dict(file=name+'.f32le',origin=origin,transient=transient,length=length,start=destination,end=destination+total))
 files['manifest.json']=(json.dumps(dict(cases=cases,decoder_integration_accepted=False,absolute_tolerance=0.000001),indent=2)+'\n').encode()
 return files
if __name__=='__main__':
 OUT.mkdir(parents=True,exist_ok=True)
 for n,b in generate().items():(OUT/n).write_bytes(b)
