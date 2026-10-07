#!/usr/bin/env python3
"""Owned AV1 masked two-reference prediction; no source media inputs."""
import argparse,hashlib,json,subprocess
from pathlib import Path
from generate_av1_show_existing_samples import sequence,webm
from generate_av1_mixed_lossless_samples import encode,key
from generate_av1_mixed_inter_samples import frame

def main():
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('--writer',type=Path,required=True)
    p.add_argument('--oracle',type=Path,required=True)
    a=p.parse_args();root=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors';records=[]
    for kind,index in [(0,i) for i in range(16)]+[(1,0)]:
      for base in ([1] if kind==0 else [1,64]):
       for sign in [0,1]:
        for mask in [1,6]:
         for groups in [15,5,10]:
          for adaptive in [False,True]:
           for residual in [1,-1]:
            for hidden_level in ([14] if kind==0 else [14,256,1024]):
             prefix,initial=encode(a.writer,base,mask,False,False,hidden_level*residual)
             other,other_map=encode(a.writer,base,mask,False,False,-hidden_level*residual)
             data=sequence(False,False,False,masked_compound=True)+key(prefix,base,False,False)+key(other,base,False,False,kind=2,refresh=128);maps=[initial,other_map]
             for current in [mask,15-mask]:
                 entropy,grid=encode(a.writer,base,current,False,adaptive,residual,inter=True,compound_mask=(kind,index,sign),masked_blocks=groups)
                 data+=frame(entropy,base,False,adaptive,1,reference_select=True);maps.append(grid)
             name=f'av1-masked-compound-k{kind}-idx{index}-sign{sign}-q{base}-mask{mask}-groups{groups}-adapt{int(adaptive)}-dc{residual}'
             if hidden_level!=14:name+=f'-level{hidden_level}'
             file=name+'.obu';expected=name+'.yuv';wrapped=name+'.webm'
             (root/file).write_bytes(data);w=webm(data);(root/wrapped).write_bytes(w)
             subprocess.run([str(a.oracle),str(root/file),'2',str(root/expected)],check=True)
             pixels=(root/expected).read_bytes();assert len(pixels)==3072
             records.append(dict(file=file,sha256=hashlib.sha256(data).hexdigest(),reference=expected,reference_sha256=hashlib.sha256(pixels).hexdigest(),webm=wrapped,webm_sha256=hashlib.sha256(w).hexdigest(),kind=kind,index=index,sign=sign,hidden_level=hidden_level,base=base,mask=mask,groups=groups,adaptive=adaptive,residual=residual,maps=maps))
    (root/'av1-masked-compound-generated.json').write_text(json.dumps(dict(fixtures=records),indent=2)+'\n')
if __name__=='__main__':main()
