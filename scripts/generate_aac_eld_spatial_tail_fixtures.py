#!/usr/bin/env python3
"""Owned ELD mixed-tail movies for bounded spatial dispatch; offline only."""
import json,struct
from pathlib import Path
from generate_aac_ld_mps_transport_fixtures import envelope,packed,field,length
from generate_he_aac_packet_fixtures import video_fixture
ROOT=Path(__file__).resolve().parents[1]
OUT=ROOT/'tests/fixtures/playback-errors/aac-eld-spatial-tail'
def raw(data):return ''.join(field(v,8)for v in data)
def wrap(data):return '0011'+length(len(data))+raw(data)
def main():
 OUT.mkdir(exist_ok=True);blob=bytearray();cases=[]
 for tree in range(8):
  source=ROOT/'tests/fixtures/playback-errors/aac-eld-mps-core'
  bodies=[(source/f'tree{tree}-{i}.spatial').read_bytes()for i in range(3)]
  cores=[]
  for i in range(3):
   n,_=struct.unpack('<II',(source/f'tree{tree}-{i}.bits').read_bytes());cores.append(raw((source/f'tree{tree}-{i}.au').read_bytes())[:n])
  modes=[('direct9',0,None),('direct12',0,None),('wrapped9',0,None),('wrapped12',0,None)]
  if tree==7:
   modes += [('length',n,None)for n in(1,14,15,269,270,65805)]
   modes += [(kind,0,error)for kind,error in [('bad-drc','AAC extension exceeds fill payload'),('nested','nested AAC EXT_DATA_LENGTH'),('overlong','AAC spatial child exceeds parent'),('empty-child','AAC spatial child exceeds parent'),('alignment',None),('orphan','continuation/stop without start')]]
  for mode,size,error in modes:
   frames=[]
   for i,core in enumerate(cores):
    tag=12 if mode.endswith('12')else 9;direct=mode.startswith('direct')
    # DRC and ancillary data precede the spatial record. Bounded unknown and
    # implicit ELD SBR children must leave later parent records accessible.
    head=raw(b'\xb0\x00\x20\x01\x5a')+wrap(b'\xa0\xff')+wrap(b'\xb0\x00')
    if not direct:head+=wrap(b'\xd0\x01')+wrap(b'\xe0\x02')+wrap(b'\x2f\xff')
    if mode=='length'and i==0:head+=wrap(b'\xf0'+bytes(size-1))
    spatial=envelope(tag,0,3,bodies[i],not direct)
    tail=''if direct else raw(b'\xb0\x00\x10\xa5')
    if mode=='bad-drc':
     middle=len(bodies[0])//2
     if i==0:spatial=envelope(9,0,2,bodies[0][:middle],True)
     elif i==1:spatial=envelope(9,0,1,bodies[0][middle:],True)
    if error and i==1:
     if mode=='bad-drc':tail=raw(b'\xb8')
     elif mode=='nested':tail=wrap(b'\x31\x00')
     elif mode=='overlong':tail='0011'+length(5)+'11110000'
     elif mode=='empty-child':tail='00110000'
     elif mode=='orphan':spatial=envelope(9,0,1,bodies[i],True)
    bits=core+head+spatial+tail;packet=bytearray(packed(bits))
    if mode=='alignment'and i==1:
     assert len(bits)%8;packet[-1]|=1
    frames.append(dict(offset=len(blob),bytes=len(packet),core_bits=len(core),end_bits=len(bits)));blob.extend(packet)
   name=f'tree{tree}-{mode}'+(f'-{size}'if mode=='length'else'')
   channels=1 if tree in(0,1,7)else 2 if tree in(2,3,4)else 6
   case=dict(name=name,tree=tree,mode=mode,length=size,error=error,error_frame=1 if error else None,asc=(source/f'tree{tree}.spatial.asc').read_bytes().hex(),channels=channels,frames=frames,slots=8,bands=64,container_rate=48000,container_frame_samples=512,pcm_offset=0,samples=1536)
   (OUT/(name+'.mp4')).write_bytes(video_fixture([case],blob,channels,write=False)['data']);cases.append(case)
 (OUT/'packets.bin').write_bytes(blob)
 (OUT/'manifest.json').write_text(json.dumps(dict(scope='Native AAC-ELD spatial playback with mixed bounded ER tails and arbitrary AAC alignment and specific malformed-tail controls.',cases=cases),indent=2)+'\n')
if __name__=='__main__':main()
