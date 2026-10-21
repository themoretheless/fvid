"""Whole-buffer scalar oracle for frame-streaming transient correction."""
import json,math,struct
from pathlib import Path
from generate_audio_ring_capacity_regressions import clip
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/eac3-transient-stream'
def generate():
 cases=[];files={}
 scenarios=[('regular',[1536]*4,[(2048,64),(0,255),None,(512,128)]),
 ('short',[256]*12,[(2048,48),None,(1024,1),None,None,None,None,(512,128),None,None,None,None]),
 ('eof-prefix',[768,512,256],[(2048,255),None,None]),
 ('mixed',[256,512,768,1536,256,768],[(1024,0),(0,255),(512,128),(2048,64),(0,255),None]),
 ('far-future',[256],[(4092,255)])]
 for name,frames,windows in scenarios:
  size=sum(frames);video,raw=clip(frames=size,bits=32,floating=True,video_frames=math.ceil(size/960))
  original=list(struct.unpack('<'+'f'*size,raw));work=[0.]*1536+original
  events=[];start=0
  for count,window in zip(frames,windows):
   if window is not None:events.append((start+window[0],window[1]))
   start+=count
  for at,length in sorted(events,key=lambda e:e[0]):
   previous=(at//256-1)*256;pn=at-previous;total=pn+length+256
   source=at-512-2*pn;destination=at-total;visible=max(0,min(at,size)-destination)
   if not visible:continue
   synthesis=work[source+1536:source+1536+visible]
   assert len(synthesis)==visible
   for i in range(visible):
    if i<256:w=(1-math.cos(math.pi*i/255))/2;value=work[destination+1536+i]*(1-w)+synthesis[i]*w
    elif i>=total-128:w=(1-math.cos(math.pi*(i-total+128)/127))/2;value=work[destination+1536+i]*w+synthesis[i]*(1-w)
    else:value=synthesis[i]
    work[destination+1536+i]=struct.unpack('<f',struct.pack('<f',value))[0]
  files[name+'-input.f32le']=raw;files[name+'.mkv']=video;files[name+'-expected.f32le']=struct.pack('<'+'f'*size,*work[1536:])
  cases.append(dict(name=name,frames=frames,windows=[None if w is None else dict(location=w[0],length=w[1])for w in windows],changes=name!='far-future'))
 files['manifest.json']=(json.dumps(dict(cases=cases,decoder_integration_accepted=False,absolute_tolerance=0.000003),indent=2)+'\n').encode()
 return files
if __name__=='__main__':
 OUT.mkdir(parents=True,exist_ok=True)
 for n,b in generate().items():(OUT/n).write_bytes(b)
