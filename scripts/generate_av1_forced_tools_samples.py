#!/usr/bin/env python3
"""Owned mixed lossless/lossy inter-prediction fixtures; no private media input."""
import argparse,hashlib,json,subprocess
from pathlib import Path
from generate_av1_show_existing_samples import Bits,obu,sequence,webm
from generate_av1_mixed_lossless_samples import encode,key

from generate_av1_mixed_inter_samples import frame

def main():
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('--writer',type=Path,required=True);p.add_argument('--oracle',type=Path,required=True);a=p.parse_args();root=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors';records=[]
    for tools in [(1,1),(2,2),(3,3),(1,2),(2,1)]:
        reference=1
        for base in [1,64]:
            for mask in [2,6]:
                for selected in [False,True]:
                    for adaptive in [False,True]:
                        for residual in [1,-1]:
                            prefix,initial=encode(a.writer,base,mask,False,False,residual)
                            other,other_map=encode(a.writer,base,mask,False,False,-residual)
                            data=sequence(False,False,False)+key(prefix,base,False,False)+key(other,base,False,False,kind=2,refresh=128);maps=[initial,other_map]
                            for current in [mask,15-mask]:
                                entropy,grid=encode(a.writer,base,current,selected,adaptive,residual,inter=True,reference=reference,forced_tools=tools)
                                data+=frame(entropy,base,selected,adaptive,reference,forced_tools=tools);maps.append(grid)
                            name=f'av1-forced-tools-s{tools[0]}-{tools[1]}-q{base}-mask{mask}-tx{int(selected)}-adapt{int(adaptive)}-dc{residual}';file=name+'.obu';expected=name+'.yuv';wrapped=name+'.webm'
                            (root/file).write_bytes(data);w=webm(data);(root/wrapped).write_bytes(w)
                            subprocess.run([str(a.oracle),str(root/file),'2',str(root/expected)],check=True)
                            pixels=(root/expected).read_bytes();assert len(pixels)==3072
                            records.append(dict(file=file,sha256=hashlib.sha256(data).hexdigest(),reference=expected,reference_sha256=hashlib.sha256(pixels).hexdigest(),webm=wrapped,webm_sha256=hashlib.sha256(w).hexdigest(),forced_tools=tools,logical_reference=reference,physical_references=[7 if logical==reference else 0 for logical in range(1,8)],base=base,mask=mask,selected=selected,adaptive=adaptive,residual=residual,maps=maps))
    (root/'av1-forced-tools-generated.json').write_text(json.dumps(dict(fixtures=records),indent=2)+'\n')
if __name__=='__main__':main()
