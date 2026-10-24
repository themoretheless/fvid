"""Owned expanded dialogue levels with RF/Line, role changes and transient history."""
import json
from pathlib import Path
from generate_eac3_dialogue_normalization import level,ROOT
OUT=ROOT/'eac3-dialogue-expanded'
def replace_frames(data,pairs):
 cursor=0;out=bytearray(data)
 for old,new in pairs:
  at=data.find(old,cursor);assert at>=cursor
  out[at:at+len(old)]=new;cursor=at+len(old)
 return bytes(out)
def generate():
 source=ROOT/'eac3-rf-expanded';m=json.loads((source/'manifest.json').read_bytes());files={};cases=[]
 for original in m['cases']:
  c=dict(original);c['levels']=[[1,27],[27,1],[0,31]]
  raw=(source/(c['name']+'.ec3')).read_bytes();pairs=[];at=0
  for f in range(3):
   for component in c['components']:
    size=((raw[at+2]&7)*256+raw[at+3]+1)*2;old=raw[at:at+size]
    pairs.append((old,level(old,*c['levels'][f])));at+=size
  assert at==len(raw)
  files[c['name']+'.ec3']=b''.join(new for old,new in pairs)
  for ext in ['mkv']+(['mp4']if 'mp4'in c else[]):files[c['name']+'.'+ext]=replace_frames((source/(c['name']+'.'+ext)).read_bytes(),pairs)
  for part in c['components']:files[part['control']]=(source/part['control']).read_bytes()
  cases.append(c)
 files['manifest.json']=(json.dumps(dict(accepted=True,cases=cases),indent=2)+'\n').encode();return files
if __name__=='__main__':
 OUT.mkdir(parents=True,exist_ok=True)
 for n,b in generate().items():(OUT/n).write_bytes(b)
