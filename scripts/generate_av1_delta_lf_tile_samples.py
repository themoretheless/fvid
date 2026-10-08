#!/usr/bin/env python3
"""Owned multi-superblock and multi-tile AV1 delta-loop-filter fixtures, generated separately from offline tests."""
import argparse,hashlib,json,subprocess,tempfile,itertools
from pathlib import Path
from generate_av1_show_existing_samples import webm

def main():
 p=argparse.ArgumentParser(description=__doc__);p.add_argument('--encoder',type=Path,required=True);p.add_argument('--oracle',type=Path,required=True);p.add_argument('--second-oracle',type=Path,required=True);a=p.parse_args()
 root=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors';records=[]
 cases=[(depth,3,True,layout,True,48 if depth==8 else 40,0) for depth in [8,10,12] for layout in [0,1]]
 cases.append((12,3,True,1,True,40,1))
 for depth,colors,chroma,layout,mirror,quality,phase in cases:
   width,height=256,128
   def plane(w,h,p):
    values=[(24+(x//4+y//4)%colors*max(1,208//(colors-1))) if p==0 else ((224-((x//2+y//2)%colors)*max(1,192//(colors-1)) if p==2 and mirror else 32+((x//2+y//2)%colors)*max(1,192//(colors-1))) if chroma else 128) for y in range(h) for x in range(w)]
    values=[max(0,min(255,v+((i*3+i//w*5)%5-2))) for i,v in enumerate(values)]
    if True:
     noise=7
     period=128 if p==0 else 64
     values=[v if ((i%w%period<period//2) != bool(phase)) else max(0,min(255,64+(i%w)*80//w+((i*17+i//w*13)%noise-noise//2))) for i,v in enumerate(values)]
    if depth==8:return bytes(values)
    return b''.join((v<<(depth-8)).to_bytes(2,'little') for v in values)
   pixels=plane(width,height,0)+plane((width+1)//2,(height+1)//2,1)+plane((width+1)//2,(height+1)//2,2)
   name=f'av1-delta-lf-tiles-depth{depth}-colors{colors}-chroma{int(chroma)}-layout{layout}-vflip{int(mirror)}-q{quality}'+(f'-phase{phase}' if phase else '');file=name+'.obu';expected=name+'.yuv';wrapped=name+'.webm'
   with tempfile.TemporaryDirectory(prefix='fvid-owned-palette-') as tmp:
    input=Path(tmp)/'owned.y4m';input.write_bytes(f'YUV4MPEG2 W{width} H{height} F50:1 Ip A1:1 C{"420jpeg" if depth==8 else "420p"+str(depth)}\nFRAME\n'.encode()+pixels)
    subprocess.run([str(a.encoder),'--obu','--cpu-used='+'0','--passes=1','--sb-size=64',f'--tile-columns={layout}',f'--tile-rows={layout}','--limit=1','--lossless=0','--end-usage=q',f'--cq-level={quality}','--deltaq-mode=2','--delta-lf-mode=1','--enable-qm=0',f'--bit-depth={depth}',f'--input-bit-depth={depth}','--min-partition-size=32','--max-partition-size=32','--enable-rect-partitions=0','--enable-ab-partitions=0','--enable-1to4-partitions=0','--tune-content='+'default','--enable-palette=1','--enable-intrabc=0','--enable-cdef=0','--enable-restoration=0','--enable-ref-frame-mvs=0',f'--output={root/file}',str(input)],check=True)
   data=(root/file).read_bytes();subprocess.run([str(a.oracle),str(root/file),'1',str(root/expected),'whole-packet'],check=True)
   golden=(root/expected).read_bytes()
   with tempfile.TemporaryDirectory(prefix='fvid-second-palette-oracle-') as tmp:
    cross=Path(tmp)/'cross.yuv';subprocess.run([str(a.second_oracle),str(root/file),str(cross)],check=True)
    assert cross.read_bytes()==golden, name+': independent oracle mismatch'
   source_exact=golden==pixels


   container=webm(data,(width,height));(root/wrapped).write_bytes(container)
   records.append(dict(file=file,sha256=hashlib.sha256(data).hexdigest(),reference=expected,reference_sha256=hashlib.sha256(golden).hexdigest(),source_sha256=hashlib.sha256(pixels).hexdigest(),source_exact=source_exact,quality=quality,oracles=['libaom','dav1d'],mirror=mirror,webm=wrapped,webm_sha256=hashlib.sha256(container).hexdigest(),depth=depth,colors=colors,chroma=chroma,layout=layout,phase=phase,tiles=1<<(2*layout),superblocks=8,size=[width,height]))
 (root/'av1-delta-lf-tiles-generated.json').write_text(json.dumps(dict(fixtures=records),indent=2)+'\n')
if __name__=='__main__':main()
