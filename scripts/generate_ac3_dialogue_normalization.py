"""Own AC-3 dialogue-level witnesses; no external encoder or private media."""
import json
from pathlib import Path
from generate_ac3_rf_compression import authored,crc_frame,mp4_movie,RATE
from generate_eac3_bsid import movie
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/ac3-dialogue-normalization'
def level(raw,first,second):
 bits=list(''.join(f'{b:08b}'for b in raw));layout=int(''.join(bits[48:51]),2)
 at=51+2*int(layout&1!=0 and layout!=1)+2*int(layout&4!=0)+2*int(layout==2)+1
 bits[at:at+5]=f'{first:05b}';at+=5
 present=int(bits[at]);at+=1+8*present
 for size in [8,7]:
  present=int(bits[at]);at+=1+size*present
 if layout==0:bits[at:at+5]=f'{second:05b}'
 return crc_frame(bytes(int(''.join(bits[i:i+8]),2)for i in range(0,len(bits),8)))
def generate():
 files={};cases=[]
 for layout,lfe in [(0,False),(0,True),(1,False),(2,False),(2,True)]:
  for r in range(3):
   width=(1 if layout==1 else 2)+int(lfe);key=f'l{layout}-{int(lfe)}-r{r}'
   quiet=authored(layout,lfe,r,None,None,control=True)
   files[key+'-control.mkv']=movie([quiet]*3,blocks=6,channels=width,rate=RATE[r],codec='A_AC3')
   for code in (range(32) if r==0 else [0,1,15,27,31]):
    name=f'{key}-d{code}';levels=[[code,31-code],[31-code,code],[0,31]]
    packets=[level(quiet,a,b)for a,b in levels]
    files[name+'.mkv']=movie(packets,blocks=6,channels=width,rate=RATE[r],codec='A_AC3')
    cases.append(dict(name=name,control=key+'-control.mkv',rate=RATE[r],layout=layout,channels=width,levels=levels))
    if code in [0,1,27,31]:files[name+'.mp4']=mp4_movie(packets,layout,lfe,r);cases[-1]['mp4']=True
 # Loud dual/stereo witnesses distinguish scaling before final clipping.
 for layout in [0,2]:
  for code in [1,27,31]:
   key=f'l{layout}-0-r0';name=f'{key}-d{code}-loud';levels=[[code,31-code],[31-code,code],[0,31]]
   loud=authored(layout,False,0,127,127,clip=True)
   quiet=authored(layout,False,0,None,None,control=True,clip=True)
   files[key+'-loud-control.mkv']=movie([quiet]*3,blocks=6,channels=2,rate=48000,codec='A_AC3')
   packets=[level(loud,a,b)for a,b in levels]
   files[name+'.mkv']=movie(packets,blocks=6,channels=2,rate=48000,codec='A_AC3')
   files[name+'.mp4']=mp4_movie(packets,layout,False,0)
   cases.append(dict(name=name,control=key+'-loud-control.mkv',rate=48000,layout=layout,channels=2,levels=levels,loud=True,mp4=True))
 files['manifest.json']=(json.dumps(dict(accepted=True,cases=cases),indent=2)+'\n').encode()
 return files
if __name__=='__main__':
 OUT.mkdir(parents=True,exist_ok=True)
 for n,b in generate().items():(OUT/n).write_bytes(b)
