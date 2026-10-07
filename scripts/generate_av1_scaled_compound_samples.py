#!/usr/bin/env python3
"""Owned AV1 scaled masked two-reference prediction; no source media inputs."""
import argparse,hashlib,json,subprocess
from pathlib import Path
from generate_av1_show_existing_samples import sequence,webm
from generate_av1_mixed_lossless_samples import encode,key
from generate_av1_mixed_inter_samples import frame

CASES=[((16,16),(32,32),(32,32)),((32,32),(16,16),(16,16)),((16,32),(32,16),(32,32)),((15,31),(31,15),(16,32))]

def main():
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('--writer',type=Path,required=True)
    p.add_argument('--oracle',type=Path,required=True)
    a=p.parse_args();root=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors';records=[]
    for case,(first_size,second_size,current_size) in enumerate(CASES):
     for kind,index in [(0,i) for i in range(16)]+[(1,0)]:
      for sign in [0,1]:
       for adaptive in [False,True]:
        for mask in [1,6]:
             base=1 if case%2==0 else 64;residual=1 if sign==0 else -1;hidden_level=256;groups=15;interpolation=(index+case)%4
             prefix,initial=encode(a.writer,base,mask,False,False,hidden_level*residual,residual_everywhere=True,frame_size=first_size)
             other,other_map=encode(a.writer,base,mask,False,False,-hidden_level*residual,residual_everywhere=True,frame_size=second_size)
             data=sequence(False,False,False,masked_compound=True)+key(prefix,base,False,False,size=first_size)+key(other,base,False,False,kind=2,refresh=128,size=second_size);maps=[initial,other_map]
             for current in [mask,15-mask]:
                 entropy,grid=encode(a.writer,base,current,False,adaptive,residual,inter=True,compound_mask=(kind,index,sign),masked_blocks=groups,frame_size=current_size)
                 data+=frame(entropy,base,False,adaptive,1,reference_select=True,size=current_size,refresh=0,interpolation=interpolation);maps.append(grid)
             name=f'av1-scaled-compound-case{case}-k{kind}-idx{index}-sign{sign}-mask{mask}-adapt{int(adaptive)}'
             file=name+'.obu';expected=name+'.yuv';wrapped=name+'.webm'
             (root/file).write_bytes(data);w=webm(data,current_size);(root/wrapped).write_bytes(w)
             subprocess.run([str(a.oracle),str(root/file),'2',str(root/expected)],check=True)
             pixels=(root/expected).read_bytes();assert len(pixels)==3*current_size[0]*current_size[1]
             records.append(dict(file=file,sha256=hashlib.sha256(data).hexdigest(),reference=expected,reference_sha256=hashlib.sha256(pixels).hexdigest(),webm=wrapped,webm_sha256=hashlib.sha256(w).hexdigest(),kind=kind,index=index,sign=sign,hidden_level=hidden_level,base=base,mask=mask,groups=groups,adaptive=adaptive,residual=residual,maps=maps,case=case,reference_sizes=[first_size,second_size],current_size=current_size,interpolation=interpolation))
    (root/'av1-scaled-compound-generated.json').write_text(json.dumps(dict(fixtures=records),indent=2)+'\n')
if __name__=='__main__':main()
