#!/usr/bin/env python3
"""Owned synthetic AV1 loop restoration refusal reproducers, generated separately from offline tests."""
import argparse,hashlib,json,subprocess,tempfile,itertools
from pathlib import Path
from generate_av1_show_existing_samples import webm

def main():
 p=argparse.ArgumentParser(description=__doc__);p.add_argument('--encoder',type=Path,required=True);p.add_argument('--oracle',type=Path,required=True);p.add_argument('--second-oracle',type=Path,required=True);a=p.parse_args()
 root=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors';records=[]
 cases=list(itertools.product([8,10,12],[3],[True],[0],[True],[32,48,56],[7],range(2)))
 for depth,colors,chroma,layout,mirror,quality,matrix,orientation in cases:
   width,height=192,128
   def plane(w,h,p):
    values=[max(0,min(255,64+(x*90//w)+(y*60//h)+(((x*17+y*31+p*11)%17)-8)*(2 if orientation==0 else 5))) for y in range(h) for x in range(w)]
    if depth==8:return bytes(values)
    return b''.join((v<<(depth-8)).to_bytes(2,'little') for v in values)
   pixels=plane(width,height,0)+plane((width+1)//2,(height+1)//2,1)+plane((width+1)//2,(height+1)//2,2)
   name=f'av1-restoration-depth{depth}-colors{colors}-chroma{int(chroma)}-layout{layout}-vflip{int(mirror)}-q{quality}-orientation{orientation}';file=name+'.obu';expected=name+'.yuv';wrapped=name+'.webm'
   with tempfile.TemporaryDirectory(prefix='fvid-owned-palette-') as tmp:
    input=Path(tmp)/'owned.y4m';input.write_bytes(f'YUV4MPEG2 W{width} H{height} F50:1 Ip A1:1 C{"420jpeg" if depth==8 else "420p"+str(depth)}\nFRAME\n'.encode()+pixels)
    subprocess.run([str(a.encoder),'--obu','--cpu-used=0','--passes=1','--sb-size=64','--limit=1','--lossless=0','--end-usage=q',f'--cq-level={quality}','--deltaq-mode=0','--loopfilter-control=0','--enable-qm=0',f'--qm-min={matrix}',f'--qm-max={matrix}',f'--bit-depth={depth}',f'--input-bit-depth={depth}','--min-partition-size=4','--max-partition-size=64','--enable-rect-partitions=1','--enable-ab-partitions=0','--enable-1to4-partitions=0','--tune-content=default','--enable-tx-size-search=1','--enable-tx64=1','--enable-rect-tx=1','--enable-palette=0','--enable-intrabc=0','--enable-cdef=0','--enable-restoration=1','--enable-ref-frame-mvs=0',f'--output={root/file}',str(input)],check=True)
   data=(root/file).read_bytes();subprocess.run([str(a.oracle),str(root/file),'1',str(root/expected),'whole-packet'],check=True)
   golden=(root/expected).read_bytes()
   with tempfile.TemporaryDirectory(prefix='fvid-second-palette-oracle-') as tmp:
    cross=Path(tmp)/'cross.yuv';subprocess.run([str(a.second_oracle),str(root/file),str(cross)],check=True)
    assert cross.read_bytes()==golden, name+': independent oracle mismatch'
   source_exact=golden==pixels


   container=webm(data,(width,height));(root/wrapped).write_bytes(container)
   records.append(dict(file=file,sha256=hashlib.sha256(data).hexdigest(),reference=expected,reference_sha256=hashlib.sha256(golden).hexdigest(),source_sha256=hashlib.sha256(pixels).hexdigest(),source_exact=source_exact,quality=quality,matrix_enabled=False,orientation=orientation,oracles=['libaom','dav1d'],mirror=mirror,webm=wrapped,webm_sha256=hashlib.sha256(container).hexdigest(),depth=depth,colors=colors,chroma=chroma,layout=layout,size=[width,height]))
 (root/'av1-restoration-generated.json').write_text(json.dumps(dict(fixtures=records),indent=2)+'\n')
if __name__=='__main__':main()
