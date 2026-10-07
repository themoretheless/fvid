#!/usr/bin/env python3
"""Owned AV1 overlapped-motion compensation fixtures, no source media."""
import argparse,hashlib,json,subprocess,itertools
from pathlib import Path
from generate_av1_show_existing_samples import sequence,webm
from generate_av1_mixed_lossless_samples import encode,key
from generate_av1_mixed_inter_samples import frame

def main():
 p=argparse.ArgumentParser(description=__doc__);p.add_argument('--writer',type=Path,required=True);p.add_argument('--oracle',type=Path,required=True);a=p.parse_args();root=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors';records=[]
 for refs,base,mask,obmc,selected,adaptive,level,residual,movement in itertools.product([(1,7),(7,1)],[1,64],[1,2,4,6],[2,4,8,14],[False,True],[False,True],[256,1024],[1,-1],[0,1]):
  prefix,initial=encode(a.writer,base,mask,False,False,level*residual,residual_everywhere=True)
  other,other_map=encode(a.writer,base,mask,False,False,-level*residual,residual_everywhere=True)
  data=sequence(False,False,False)+key(prefix,base,False,False)+key(other,base,False,False,kind=2,refresh=128);maps=[initial,other_map];previous=None
  for frame_index,current in enumerate([mask,15-mask]):
   entropy,grid=encode(a.writer,base,current,selected,adaptive,residual,inter=True,forced_reference=True,forced_tools=(2,2),segment_references=refs,obmc_blocks=obmc)
   models=[(0,[0,0,65536,0,0,65536]) for _ in range(7)]
   if movement:
    direction=1 if frame_index==0 else -1
    models[0]=(1,[direction*16384,-direction*32768,65536,0,0,65536])
    models[6]=(1,[-direction*32768,direction*16384,65536,0,0,65536])
   data+=frame(entropy,base,selected,adaptive,1,forced_reference=True,forced_tools=(2,2),segment_references=refs,motion_switchable=True,global_models=models,previous_globals=previous);maps.append(grid);previous=[v[1] for v in models]
  name=f'av1-obmc-refs{refs[0]}{refs[1]}-q{base}-mask{mask}-blocks{obmc}-tx{int(selected)}-adapt{int(adaptive)}-level{level}-dc{residual}'
  if movement:name+='-fractional'
  file=name+'.obu';expected=name+'.yuv';wrapped=name+'.webm'
  (root/file).write_bytes(data);w=webm(data);(root/wrapped).write_bytes(w)
  subprocess.run([str(a.oracle),str(root/file),'2',str(root/expected)],check=True)
  pixels=(root/expected).read_bytes();assert len(pixels)==3072
  records.append(dict(file=file,sha256=hashlib.sha256(data).hexdigest(),reference=expected,reference_sha256=hashlib.sha256(pixels).hexdigest(),webm=wrapped,webm_sha256=hashlib.sha256(w).hexdigest(),refs=refs,base=base,mask=mask,obmc=obmc,selected=selected,adaptive=adaptive,level=level,residual=residual,movement=movement,maps=maps))
 (root/'av1-obmc-generated.json').write_text(json.dumps(dict(fixtures=records),indent=2)+'\n')
if __name__=='__main__':main()
