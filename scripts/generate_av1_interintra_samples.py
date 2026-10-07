#!/usr/bin/env python3
"""Owned AV1 inter-intra mode and wedge fixtures; no private media input."""
import argparse,hashlib,json,subprocess
from pathlib import Path
from generate_av1_show_existing_samples import Bits,obu,sequence,webm
from generate_av1_mixed_lossless_samples import encode,key

from generate_av1_mixed_inter_samples import frame

def main():
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('--writer',type=Path,required=True);p.add_argument('--oracle',type=Path,required=True);a=p.parse_args();root=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors';records=[]
    for model_index in range(68):
      mode=model_index%4;wedge=None if model_index<4 else (model_index-4)//4
      kind,params,interpolation=0,[0,0,65536,0,0,65536],0
      for reference in [1,7]:
        for base in ([1,64] if wedge is None else [1]):
            for mask in [1,6]:
                for selected in [False]:
                    for adaptive in [False,True]:
                        for residual in [1,-1]:
                            prefix,initial=encode(a.writer,base,mask,False,False,14 if residual>0 else -14)
                            other,other_map=encode(a.writer,base,mask,False,False,-14 if residual>0 else 14)
                            data=sequence(False,False,False,interintra=True)+key(prefix,base,False,False)+key(other,base,False,False,kind=2,refresh=128);maps=[initial,other_map];global_frames=[];previous=None
                            for frame_index,current in enumerate([mask,15-mask]):
                                values=params if frame_index==0 else [-params[0],-params[1],131072-params[2],-params[3],-params[4],131072-params[5]]
                                global_models=[(0,[0,0,65536,0,0,65536]) for _ in range(7)];global_models[reference-1]=(kind,values)
                                global_frames.append(global_models)
                                entropy,grid=encode(a.writer,base,current,selected,adaptive,residual,inter=True,reference=reference,forced_reference=True,forced_tools=(0,0),switchable_filter=False,interintra_mode=mode,interintra_wedge=wedge)
                                data+=frame(entropy,base,selected,adaptive,reference,forced_reference=True,forced_tools=(0,0),global_models=global_models,previous_globals=previous if reference==1 else None,interpolation=interpolation);maps.append(grid);previous=[m[1] for m in global_models]
                            name=f'av1-interintra-m{model_index}-ref{reference}-q{base}-mask{mask}-tx{int(selected)}-adapt{int(adaptive)}-dc{residual}';file=name+'.obu';expected=name+'.yuv';wrapped=name+'.webm'
                            (root/file).write_bytes(data);w=webm(data);(root/wrapped).write_bytes(w)
                            subprocess.run([str(a.oracle),str(root/file),'2',str(root/expected)],check=True)
                            pixels=(root/expected).read_bytes();assert len(pixels)==3072
                            records.append(dict(file=file,sha256=hashlib.sha256(data).hexdigest(),reference=expected,reference_sha256=hashlib.sha256(pixels).hexdigest(),webm=wrapped,webm_sha256=hashlib.sha256(w).hexdigest(),global_frames=global_frames,interpolation=interpolation,mode=mode,wedge=wedge,model_kind=kind,logical_reference=reference,physical_references=[7 if logical==reference else 0 for logical in range(1,8)],base=base,mask=mask,selected=selected,adaptive=adaptive,residual=residual,maps=maps))
    (root/'av1-interintra-generated.json').write_text(json.dumps(dict(fixtures=records),indent=2)+'\n')
if __name__=='__main__':main()
