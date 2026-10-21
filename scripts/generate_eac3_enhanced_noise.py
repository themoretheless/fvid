"""Owned enhanced coupling noise, per-channel flags and amplitude controls."""
import json
from pathlib import Path
from generate_eac3_enhanced_coupling import packet,geometry,SUB,DEFAULT,amplitude
from generate_eac3_zero_bit import PATTERNS
from generate_eac3_bsid import movie
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/eac3-enhanced-noise'

def generate():
 files={};cases=[];covered=set()
 for exponent in [8,12]:
  for converted in [False,True]:
   for begin in [0,8]:
    for merged in [False,True]:
     for aht in [False,True]:
      for index,(pattern,flags) in enumerate(PATTERNS.items()):
       seed=(index*5+begin+int(merged)*7+int(aht)*3+int(converted)*11)%32
       lo,hi,start,stop=geometry(begin);gains=[[],[]];codes=[[],[]];band=-1
       for sb in range(lo,hi):
        if sb==lo or not(merged and DEFAULT[sb]):band+=1
        for ch in range(2):
         code=(seed+ch*7+band*3)%32;covered.add(code)
         gains[ch].extend([amplitude(code)]*(SUB[sb+1]-SUB[sb]))
         codes[ch].extend([code]*(SUB[sb+1]-SUB[sb]))
       name=f'c-e{exponent}-t{2 if converted else 0}-b{begin}-g{int(merged)}-a{int(aht)}-{pattern}'
       args=(converted,begin,seed,merged,aht,None)
       raw=packet(*args,zero=True,flags=flags,exponent=exponent)
       unity=packet(*args,zero=True,flags=flags,exponent=exponent,unity=True)
       files[name+'.ec3']=raw*3;files[name+'.mkv']=movie([raw]*3);files[name+'-unity.ec3']=unity
       cases.append(dict(name=name,file=name+'.ec3',video=name+'.mkv',unity=name+'-unity.ec3',blocks=6,channels=2,mask=4 if aht else 0,exponent=exponent,allocated=False,pattern=pattern,flags=flags,start=start,end=stop,codes=codes,gains=gains))
 assert covered==set(range(32)),covered
 files['manifest.json']=(json.dumps(dict(cases=cases,covered_codes=sorted(covered)),indent=2)+'\n').encode()
 return files

if __name__=='__main__':
 OUT.mkdir(parents=True,exist_ok=True)
 for name,data in generate().items():(OUT/name).write_bytes(data)
