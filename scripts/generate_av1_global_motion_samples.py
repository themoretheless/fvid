#!/usr/bin/env python3
"""Owned mixed lossless/lossy inter-prediction fixtures; no private media input."""
import argparse,hashlib,json,subprocess
from pathlib import Path
from generate_av1_show_existing_samples import Bits,obu,sequence,webm
from generate_av1_mixed_lossless_samples import encode,key

from generate_av1_mixed_inter_samples import frame

def main():
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('--writer',type=Path,required=True);p.add_argument('--oracle',type=Path,required=True);a=p.parse_args();root=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors';records=[]
    models=[(1,[65536,0,65536,0,0,65536]),(1,[0,-32768,65536,0,0,65536]),(1,[32768,-65536,65536,0,0,65536]),(2,[0,0,65536,1024,-1024,65536]),(2,[16384,-32768,66560,0,0,66560]),(3,[32768,0,66048,1024,-768,65280]),(3,[0,65536,65024,-1024,512,66048]),(3,[0,0,73728,8192,8192,73728])]
    choices=[(kind,params,0) for kind,params in models]+[(kind,params,4) for kind,params in models[:3]+models[-1:]]
    for model_index,(kind,params,interpolation) in enumerate(choices):
      for reference in [1,7]:
        for base in [1,64]:
            for mask in [2,6]:
                for selected in [False,True]:
                    for adaptive in [False,True]:
                        for residual in [1,-1]:
                            prefix,initial=encode(a.writer,base,mask,False,False,14 if residual>0 else -14)
                            other,other_map=encode(a.writer,base,mask,False,False,-14 if residual>0 else 14)
                            data=sequence(False,False,False)+key(prefix,base,False,False)+key(other,base,False,False,kind=2,refresh=128);maps=[initial,other_map];global_frames=[];previous=None
                            for frame_index,current in enumerate([mask,15-mask]):
                                values=params if frame_index==0 else [-params[0],-params[1],131072-params[2],-params[3],-params[4],131072-params[5]]
                                global_models=[(0,[0,0,65536,0,0,65536]) for _ in range(7)];global_models[reference-1]=(kind,values)
                                global_frames.append(global_models)
                                entropy,grid=encode(a.writer,base,current,selected,adaptive,residual,inter=True,reference=reference,forced_reference=True,forced_tools=(2,2),switchable_filter=interpolation==4 and kind==1)
                                data+=frame(entropy,base,selected,adaptive,reference,forced_reference=True,forced_tools=(2,2),global_models=global_models,previous_globals=previous if reference==1 else None,interpolation=interpolation);maps.append(grid);previous=[m[1] for m in global_models]
                            name=f'av1-global-motion-m{model_index}-ref{reference}-q{base}-mask{mask}-tx{int(selected)}-adapt{int(adaptive)}-dc{residual}';file=name+'.obu';expected=name+'.yuv';wrapped=name+'.webm'
                            (root/file).write_bytes(data);w=webm(data);(root/wrapped).write_bytes(w)
                            subprocess.run([str(a.oracle),str(root/file),'2',str(root/expected)],check=True)
                            pixels=(root/expected).read_bytes();assert len(pixels)==3072
                            records.append(dict(file=file,sha256=hashlib.sha256(data).hexdigest(),reference=expected,reference_sha256=hashlib.sha256(pixels).hexdigest(),webm=wrapped,webm_sha256=hashlib.sha256(w).hexdigest(),global_frames=global_frames,interpolation=interpolation,model_kind=kind,logical_reference=reference,physical_references=[7 if logical==reference else 0 for logical in range(1,8)],base=base,mask=mask,selected=selected,adaptive=adaptive,residual=residual,maps=maps))
    (root/'av1-global-motion-generated.json').write_text(json.dumps(dict(fixtures=records),indent=2)+'\n')
if __name__=='__main__':main()
