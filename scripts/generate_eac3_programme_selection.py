"""Owned eight-programme selection videos, without external codec tools."""
import json
from pathlib import Path
from generate_eac3_dependent import packet,COUNTS
from generate_eac3_bsid import movie
from generate_eac3_playback import patch_crc
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/eac3-programme-selection'
RATES=[48000,44100,32000]
def identify(raw,identity):
 data=bytearray(raw);data[2]=(data[2]&0xc7)|(identity<<3);return patch_crc(data)
def generate():
 files={};cases=[]
 def store(name,programmes,controls,rate,blocks,width,expanded):
  groups=[b''.join(programmes[i][f]for i in range(8))for f in range(3)]
  files[name+'.ec3']=b''.join(groups);files[name+'.mkv']=movie(groups,blocks=blocks,channels=width,rate=rate)
  names=[]
  for i,frames in enumerate(controls):
   key=f'{name}-p{i}-control';files[key+'.ec3']=b''.join(frames);files[key+'.mkv']=movie(frames,blocks=blocks,channels=width,rate=rate);names.append(key)
  cases.append(dict(name=name,controls=names,rate=rate,blocks=blocks,channels=width,expanded=expanded))
 for layout in range(8):
  for lfe in [False,True]:
   for r,rate in enumerate(RATES):
    for blocks in ([3,6]if COUNTS[layout]<=2 else[3]):
     programmes=[];controls=[]
     for i in range(8):
      args=dict(blocks_override=blocks,divisor=1<<i,rate_code=r,gains=[64]+[None]*(blocks-1),gains_second=[192]+[None]*(blocks-1)if layout==0 else None,compression=16+i,compression_second=240+i if layout==0 else None)
      raw,_=packet(layout,lfe,0,0,'mixed',None,False,**args)
      controls.append([raw]*3);programmes.append([identify(raw,i)]*3)
     store(f'native-l{layout}-{int(lfe)}-r{r}-b{blocks}',programmes,controls,rate,blocks,COUNTS[layout]+int(lfe),False)
 for r,rate in enumerate(RATES):
  for blocks in [3,6]:
   for transient in [False,True]:
    programmes=[];controls=[]
    for i in range(8):
     frames=[];selected=[]
     for f in range(3):
      base,_=packet(7,True,0,0,'mixed',None,False,blocks_override=blocks,rate_code=r,divisor=1<<i,allocated=blocks!=6,gains=[96]+[None]*(blocks-1),compression=96)
      layout=2 if f==1 else 0
      dep,_=packet(layout,False,0,0,'mixed',0x0200,True,blocks_override=blocks,rate_code=r,divisor=1<<(i+1),gains=[64]+[None]*(blocks-1),gains_second=[192]+[None]*(blocks-1)if layout==0 else None,compression=16+i,compression_second=240+i if layout==0 else None,transient_windows=[(0,255),(240,127)]if transient else None)
      frames.append(identify(base,i)+dep);selected.append(base+dep)
     programmes.append(frames);controls.append(selected)
    store(f'expanded-r{r}-b{blocks}-t{int(transient)}',programmes,controls,rate,blocks,8,True)
 files['manifest.json']=(json.dumps(dict(accepted=True,cases=cases),indent=2)+'\n').encode();return files
if __name__=='__main__':
 OUT.mkdir(parents=True,exist_ok=True)
 for n,b in generate().items():(OUT/n).write_bytes(b)
