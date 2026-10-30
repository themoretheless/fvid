#!/usr/bin/env python3
"""Owned AAC core/spatial scheduling movies; no external codec or network."""
import json,struct
from pathlib import Path
from generate_aac_ld_mps_transport_fixtures import envelope,packed
from generate_he_aac_packet_fixtures import video_fixture
ROOT=Path(__file__).resolve().parents[1]
OUT=ROOT/'tests/fixtures/playback-errors/aac-ld-mps-session'
def main():
 source=ROOT/'tests/fixtures/playback-errors/aac-eld-mps-core'
 blob=bytearray();cases=[]
 for tree in range(8):
  bodies=[(source/f'tree{tree}-{i}.spatial').read_bytes()for i in range(3)]
  cores=[]
  for i in range(3):
   n,_=struct.unpack('<II',(source/f'tree{tree}-{i}.bits').read_bytes())
   cores.append(''.join(format(v,'08b')for v in(source/f'tree{tree}-{i}.au').read_bytes())[:n])
  for mode in ['immediate','late','fragments']+(['malformed-late','unfinished']if tree==7 else[]):
   wire=[envelope(9,0,3,b,True)for b in bodies]
   if mode in('late','malformed-late'):
    if mode=='malformed-late':wire[-1]=envelope(9,0,3,bodies[-1]+b'\xa5',True)
    suffixes=['','', ''.join(wire)]
   elif mode=='fragments':
    b=bodies[0];n=len(b);parts=[b[:n//3],b[n//3:2*n//3],b[2*n//3:]]
    suffixes=[envelope(9,0,seg,part,True)for seg,part in zip([2,0,1],parts)]
    suffixes[-1]+=''.join(wire[1:])
   elif mode=='unfinished':suffixes=['','','']
   else:suffixes=wire
   frames=[]
   for i,(core,suffix)in enumerate(zip(cores,suffixes)):
    packet=packed(core+suffix)
    frames.append(dict(offset=len(blob),bytes=len(packet),core_bits=len(core),end_bits=len(core+suffix),pts=i*512,duration=512));blob.extend(packet)
   channels=1 if tree in(0,1,7)else 2 if tree in(2,3,4)else 6
   name=f'tree{tree}-{mode}';asc=(source/f'tree{tree}.spatial.asc').read_bytes().hex()
   case=dict(name=name,tree=tree,mode=mode,asc=asc,channels=channels,frames=frames,slots=8,bands=64,container_rate=48000,container_frame_samples=512,pcm_offset=0,samples=1536)
   movie=video_fixture([case],blob,channels,write=False)['data']
   OUT.mkdir(exist_ok=True);(OUT/(name+'.mp4')).write_bytes(movie);cases.append(case)
 (OUT/'packets.bin').write_bytes(blob)
 (OUT/'manifest.json').write_text(json.dumps(dict(scope='Native AAC-ELD spatial playback with immediate, late and fragmented parameters; malformed tail and unmatched PCM refuse specifically.',cases=cases),indent=2)+'\n')
if __name__=='__main__':main()
