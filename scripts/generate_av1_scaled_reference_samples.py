#!/usr/bin/env python3
"""Owned AV1 inter prediction across reference sizes, without source media."""
import argparse,hashlib,itertools,json,subprocess
from pathlib import Path
from generate_av1_show_existing_samples import sequence,webm
from generate_av1_mixed_lossless_samples import encode,key
from generate_av1_mixed_inter_samples import frame

CASES=[((16,16),(32,32)),((15,15),(32,32)),((13,16),(32,32)),((16,13),(32,32)),((32,32),(16,16)),((31,31),(16,16)),((29,32),(16,16)),((32,29),(16,16)),((16,32),(32,16)),((32,16),(16,32)),((15,31),(32,16)),((31,15),(16,32))]
def stream(writer,reference_size,current_size,logical,base,mask,selected,adaptive,level,movement,interpolation,residual):
    entropy,initial=encode(writer,base,0,False,False,level*residual,residual_everywhere=True,frame_size=reference_size)
    data=sequence(False,False,False)+key(entropy,base,False,False,size=reference_size);maps=[initial]
    for frame_index,current in enumerate([mask,15-mask]):
        models=[(0,[0,0,65536,0,0,65536]) for _ in range(7)]
        if movement:
            direction=1 if frame_index==0 else -1
            models[logical-1]=(1,[direction*16384,-direction*32768,65536,0,0,65536])
        entropy,grid=encode(writer,base,current,selected,adaptive,residual,inter=True,reference=logical,forced_reference=True,forced_tools=(2,2),frame_size=current_size)
        data+=frame(entropy,base,selected,adaptive,logical,forced_reference=True,forced_tools=(2,2),global_models=models,interpolation=interpolation,size=current_size,refresh=0);maps.append(grid)
    return data,maps

def main():
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('--writer',type=Path,required=True);p.add_argument('--oracle',type=Path,required=True);a=p.parse_args();root=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors';records=[]
    for case,logical,interpolation,movement,adaptive,mask in itertools.product(range(len(CASES)),[1,7],range(4),[0,1],[False,True],[0,1]):
        reference_size,current_size=CASES[case];base=1 if case%2==0 else 64;selected=bool(interpolation%2);level=14 if movement==0 else 64;residual=1 if logical==1 else -1
        data,maps=stream(a.writer,reference_size,current_size,logical,base,mask,selected,adaptive,level,movement,interpolation,residual)
        name=f'av1-scaled-reference-case{case}-ref{logical}-filter{interpolation}-mv{movement}-adapt{int(adaptive)}-mask{mask}';file=name+'.obu';expected=name+'.yuv';wrapped=name+'.webm'
        (root/file).write_bytes(data);container=webm(data,current_size);(root/wrapped).write_bytes(container)
        subprocess.run([str(a.oracle),str(root/file),'2',str(root/expected)],check=True)
        pixels=(root/expected).read_bytes();assert len(pixels)==3*current_size[0]*current_size[1]
        records.append(dict(file=file,sha256=hashlib.sha256(data).hexdigest(),reference=expected,reference_sha256=hashlib.sha256(pixels).hexdigest(),webm=wrapped,webm_sha256=hashlib.sha256(container).hexdigest(),reference_size=reference_size,current_size=current_size,case=case,logical=logical,interpolation=interpolation,movement=movement,adaptive=adaptive,mask=mask,base=base,selected=selected,level=level,residual=residual,maps=maps))
    data,_=stream(a.writer,(31,31),(13,13),1,1,0,False,False,14,0,0,1)
    file='av1-scaled-reference-invalid-ratio.obu';(root/file).write_bytes(data)
    refusal=dict(file=file,sha256=hashlib.sha256(data).hexdigest(),error='AV1 reference scaling ratio out of range')
    (root/'av1-scaled-reference-generated.json').write_text(json.dumps(dict(fixtures=records,refusals=[refusal]),indent=2)+'\n')
if __name__=='__main__':main()
