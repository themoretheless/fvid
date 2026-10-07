#!/usr/bin/env python3
"""Owned scaled AV1 inter-intra fixtures; no private media or FFmpeg inputs."""
import argparse,hashlib,json,subprocess
from pathlib import Path
from generate_av1_show_existing_samples import sequence,webm
from generate_av1_mixed_lossless_samples import encode,key
from generate_av1_mixed_inter_samples import frame
from generate_av1_scaled_reference_samples import CASES

def main():
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('--writer',type=Path,required=True)
    p.add_argument('--oracle',type=Path,required=True)
    a=p.parse_args();root=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors';records=[]
    for case,(reference_size,current_size) in enumerate(CASES):
      for model in range(68):
       for logical in [1,7]:
        mode=model%4;wedge=None if model<4 else (model-4)//4
        base=1 if case%2==0 else 64;adaptive=bool((case+model)%2)
        mask=1 if logical==1 else 6;residual=1 if logical==1 else -1
        interpolation=(case+model)%4
        entropy,initial=encode(a.writer,base,mask,False,False,64*residual,residual_everywhere=True,frame_size=reference_size)
        data=sequence(False,False,False,interintra=True)+key(entropy,base,False,False,size=reference_size);maps=[initial]
        for current in [mask,15-mask]:
            entropy,grid=encode(a.writer,base,current,False,adaptive,residual,inter=True,reference=logical,forced_reference=True,interintra_mode=mode,interintra_wedge=wedge,frame_size=current_size)
            data+=frame(entropy,base,False,adaptive,logical,forced_reference=True,interpolation=interpolation,size=current_size,refresh=0);maps.append(grid)
        name=f'av1-scaled-interintra-case{case}-model{model}-ref{logical}'
        file=name+'.obu';expected=name+'.yuv';wrapped=name+'.webm'
        (root/file).write_bytes(data);container=webm(data,current_size);(root/wrapped).write_bytes(container)
        subprocess.run([str(a.oracle),str(root/file),'2',str(root/expected)],check=True)
        pixels=(root/expected).read_bytes();assert len(pixels)==3*current_size[0]*current_size[1]
        records.append(dict(file=file,sha256=hashlib.sha256(data).hexdigest(),reference=expected,reference_sha256=hashlib.sha256(pixels).hexdigest(),webm=wrapped,webm_sha256=hashlib.sha256(container).hexdigest(),case=case,mode=mode,wedge=wedge,logical=logical,reference_size=reference_size,current_size=current_size,maps=maps,adaptive=adaptive,interpolation=interpolation,residual=residual))
    (root/'av1-scaled-interintra-generated.json').write_text(json.dumps(dict(fixtures=records),indent=2)+'\n')
if __name__=='__main__':main()
