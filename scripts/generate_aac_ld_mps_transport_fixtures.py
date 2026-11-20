#!/usr/bin/env python3
"""Owned AAC ER/MPS envelope controls; offline, no codec or network."""
import json,pathlib
ROOT=pathlib.Path(__file__).resolve().parents[1]
OUT=ROOT/'tests/fixtures/playback-errors/aac-ld-mps-transport'
def field(x,n):return format(x,f'0{n}b')
def length(n):return field(n,4)if n<15 else field(15,4)+field(n-15,8)if n<270 else field(15,4)+field(255,8)+field(n-270,16)
def packed(s):return int(s+'0'*(-len(s)%8),2).to_bytes((len(s)+7)//8,'big')
def envelope(tag,kind,segment,body,wrapped):
 bits=field(tag,4)+field(kind,2)+field(segment,2)+''.join(field(v,8)for v in body)
 return ('0011'+length(len(bits)//8)+bits)if wrapped else bits

def main():
 blob=bytearray();cases=[]
 for tag in (9,12):
  for kind in (0,1):
   for n in (0,1,14,15,268,269,65804):
    payload=bytes((i*37+n*3+kind)%251 for i in range(n))
    for wrapped in (False,True):
     for pattern in range(4):
      parts=[(3,payload)]if pattern==0 else[(2,payload[:n//2]),(1,payload[n//2:])]if pattern==1 else[(2,payload[:n//3]),(0,payload[n//3:2*n//3]),(1,payload[2*n//3:])]if pattern==2 else[(2,b'ignored'),(3,payload)]
      fragments=[]
      for segment,body in parts:
       bits=envelope(tag,kind,segment,body,wrapped);data=packed(bits);fragments.append(dict(offset=len(blob),bits=len(bits),bytes=len(data),complete=segment in(1,3)));blob.extend(data)
      cases.append(dict(tag=tag,kind=kind,length=n,wrapped=wrapped,pattern=pattern,fragments=fragments))
 programmes=[]
 base=ROOT/'tests/fixtures/playback-errors/aac-ld-mps-assemble';pcm=ROOT/'tests/fixtures/playback-errors/aac-ld-mps-pcm'
 spatial=json.loads((base/'manifest.json').read_text());render=json.loads((pcm/'manifest.json').read_text());packets=(base/'packets.bin').read_bytes()
 for index,c in enumerate(render['cases']):
  frames=[]
  for f in spatial['cases'][c['base_case']]['frames']:
   payload=packets[f['offset']:f['offset']+f['bytes']];n=len(payload);chunks=[payload[:n//3],payload[n//3:2*n//3],payload[2*n//3:]];fragments=[]
   for part,(segment,body)in enumerate(zip([2,0,1],chunks)):
    bits=envelope(9,0,segment,body,part!=0);data=packed(bits);fragments.append(dict(offset=len(blob),bits=len(bits),bytes=len(data)));blob.extend(data)
   frames.append(dict(fragments=fragments))
  programmes.append(dict(pcm_case=index,frames=frames))
 OUT.mkdir(parents=True,exist_ok=True);(OUT/'envelopes.bin').write_bytes(blob);(OUT/'manifest.json').write_text(json.dumps(dict(scope='Transport envelope and opaque message reassembly only; no AAC/spatial movie acceptance.',cases=cases,programmes=programmes),separators=(',',':'))+'\n');print(len(cases),len(blob))
if __name__=='__main__':main()
