"""Owned E-AC-3 BSI dialogue levels; reuse only synthetic repository waveforms."""
import json
from pathlib import Path
from generate_eac3_playback import patch_crc
from generate_eac3_bsid import movie
ROOT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors'
OUT=ROOT/'eac3-dialogue-normalization'
def level(raw,a,b):
 bits=list(''.join(f'{v:08b}'for v in raw));assert int(''.join(bits[32:34]),2)!=3
 layout=int(''.join(bits[36:39]),2);bits[45:50]=f'{a:05b}'
 if layout==0:
  at=51+8*int(bits[50]);bits[at:at+5]=f'{b:05b}'
 return patch_crc(bytearray(int(''.join(bits[i:i+8]),2)for i in range(0,len(bits),8)))
def generate():
 source=ROOT/'eac3-rf-compression';m=json.loads((source/'manifest.json').read_bytes());files={};cases=[]
 for original in m['cases']:
  if original['words'][0]!=127:continue
  c=dict(original);raw=(source/(c['name']+'.ec3')).read_bytes();size=4096
  assert len(raw)==3*size
  files[c['control']]=(source/c['control']).read_bytes()
  for code in (range(32)if c['layout']in[0,2]and not c['lfe']and not c['transient']else[0,1,27,31]):
   case=dict(c);case['name']=c['name']+f'-d{code}';levels=[[code,31-code],[31-code,code],[0,31]];case['levels']=levels
   frames=[level(raw[i*size:(i+1)*size],a,b)for i,(a,b)in enumerate(levels)]
   files[case['name']+'.ec3']=b''.join(frames);files[case['name']+'.mkv']=movie(frames,blocks=3,channels=c['channels'])
   cases.append(case)
 files['manifest.json']=(json.dumps(dict(accepted=True,cases=cases),indent=2)+'\n').encode();return files
if __name__=='__main__':
 OUT.mkdir(parents=True,exist_ok=True)
 for n,b in generate().items():(OUT/n).write_bytes(b)
