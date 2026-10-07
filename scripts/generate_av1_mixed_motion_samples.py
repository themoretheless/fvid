#!/usr/bin/env python3
"""Owned mixed lossless/lossy inter-prediction fixtures; no private media input."""
import argparse,hashlib,json,subprocess,tempfile
from pathlib import Path
from generate_av1_show_existing_samples import sequence,webm
from generate_av1_mixed_lossless_samples import encode,key

from generate_av1_mixed_inter_samples import frame

def main():
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('--writer',type=Path,required=True);p.add_argument('--oracle',type=Path,required=True);a=p.parse_args();root=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors';records=[]
    for motion in [-8,-4,-2,2,4,8]:
      for reference in [1,7]:
        for base in [1,64]:
            for mask in [2,6]:
                for selected in [False,True]:
                    for adaptive in [False,True]:
                        for residual in [1,-1]:
                            prefix,initial=encode(a.writer,base,mask,False,False,14 if residual>0 else -14)
                            other,other_map=encode(a.writer,base,mask,False,False,-14 if residual>0 else 14)
                            data=sequence(False,False,False)+key(prefix,base,False,False)+key(other,base,False,False,kind=2,refresh=128);stationary=data;maps=[initial,other_map]
                            for current in [mask,15-mask]:
                                entropy,grid=encode(a.writer,base,current,selected,adaptive,residual,inter=True,reference=reference,motion=motion)
                                data+=frame(entropy,base,selected,adaptive,reference);maps.append(grid)
                                control,_=encode(a.writer,base,current,selected,adaptive,residual,inter=True,reference=reference)
                                stationary+=frame(control,base,selected,adaptive,reference)
                            name=f'av1-mixed-motion-mv{motion}-ref{reference}-q{base}-mask{mask}-tx{int(selected)}-adapt{int(adaptive)}-dc{residual}';file=name+'.obu';expected=name+'.yuv';wrapped=name+'.webm'
                            (root/file).write_bytes(data);w=webm(data);(root/wrapped).write_bytes(w)
                            subprocess.run([str(a.oracle),str(root/file),'2',str(root/expected)],check=True)
                            pixels=(root/expected).read_bytes();assert len(pixels)==3072
                            with tempfile.TemporaryDirectory() as tmp:
                                control=Path(tmp)/'stationary.obu';result=Path(tmp)/'stationary.yuv';control.write_bytes(stationary)
                                subprocess.run([str(a.oracle),str(control),'2',str(result)],check=True)
                                stationary_pixels=result.read_bytes();assert pixels[:1536]!=stationary_pixels[:1536],name
                            records.append(dict(file=file,sha256=hashlib.sha256(data).hexdigest(),reference=expected,reference_sha256=hashlib.sha256(pixels).hexdigest(),webm=wrapped,webm_sha256=hashlib.sha256(w).hexdigest(),motion=motion,reference_dc=14,stationary_sha256=hashlib.sha256(stationary_pixels).hexdigest(),logical_reference=reference,physical_references=[7 if logical==reference else 0 for logical in range(1,8)],base=base,mask=mask,selected=selected,adaptive=adaptive,residual=residual,maps=maps))
    (root/'av1-mixed-motion-generated.json').write_text(json.dumps(dict(fixtures=records),indent=2)+'\n')
if __name__=='__main__':main()
