#!/usr/bin/env python3
"""Owned synthetic full-chroma AV1 intra block copy fixtures, generated separately from offline tests."""
import argparse,hashlib,json,subprocess,tempfile,itertools
from pathlib import Path
from generate_av1_show_existing_samples import webm

def main():
 p=argparse.ArgumentParser(description=__doc__);p.add_argument('--encoder',type=Path,required=True);p.add_argument('--oracle',type=Path,required=True);p.add_argument('--second-oracle',type=Path,required=True);p.add_argument('--control',action='store_true');a=p.parse_args();prefix='av1-chroma-intrabc-control' if a.control else 'av1-chroma-intrabc'
 root=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors';records=[]
 for depth,layout,lossless,sb in itertools.product([8,10,12],["422","444"],[True,False],[64,128]):
   sx,sy=(1,0) if layout=="422" else (0,0)
   orientation=0
   width,height=769,257
   def plane(w,h,p):
    tile_w,tile_h=(63,31) if p==0 else (32,16)
    values=[32+(((x%tile_w)*17+(y%tile_h)*31+((x%tile_w)*(y%tile_h)%23)*13+p*43+orientation*19)%192) for y in range(h) for x in range(w)]
    if depth==8:return bytes(values)
    return b''.join((v<<(depth-8)).to_bytes(2,'little') for v in values)
   pixels=plane(width,height,0)+plane((width+(1<<sx)-1)>>sx,(height+(1<<sy)-1)>>sy,1)+plane((width+(1<<sx)-1)>>sx,(height+(1<<sy)-1)>>sy,2)
   name=f'{prefix}-layout{layout}-depth{depth}-lossless{int(lossless)}-sb{sb}';file=name+'.obu';expected=name+'.yuv';wrapped=name+'.webm'
   with tempfile.TemporaryDirectory(prefix='fvid-owned-intrabc-') as tmp:
    input=Path(tmp)/'owned.y4m';input.write_bytes(f'YUV4MPEG2 W{width} H{height} F50:1 Ip A1:1 C{layout if depth==8 else layout+"p"+str(depth)}\nFRAME\n'.encode()+pixels)
    subprocess.run([str(a.encoder),'--obu','--cpu-used=0','--passes=1','--limit=1',f'--lossless={int(lossless)}',*([f'--profile={1 if layout=="444" else 2}'] if depth<12 else []),'--end-usage=q','--cq-level=32','--lag-in-frames=0',f'--sb-size={sb}','--tile-columns=1',f'--bit-depth={depth}',f'--input-bit-depth={depth}','--min-partition-size=4','--max-partition-size=32','--enable-rect-partitions=0','--enable-ab-partitions=0','--enable-1to4-partitions=0','--tune-content=screen','--enable-palette=0',f'--enable-intrabc={int(not a.control)}','--enable-cdef=0','--enable-restoration=0','--enable-ref-frame-mvs=0',f'--output={root/file}',str(input)],check=True)
   data=(root/file).read_bytes();subprocess.run([str(a.oracle),'--i420','--rawvideo',f'--output-bit-depth={depth}',f'--output={root/expected}',str(root/file)],check=True)
   golden=(root/expected).read_bytes()
   with tempfile.TemporaryDirectory(prefix='fvid-second-intrabc-oracle-') as tmp:
    cross=Path(tmp)/'cross.yuv';subprocess.run([str(a.second_oracle),'--demuxer','section5','--muxer','yuv','-i',str(root/file),'-o',str(cross)],check=True)
    assert cross.read_bytes()==golden, name+': independent oracle mismatch'
   source_exact=golden==pixels
   # Odd Y4M chroma sampling may differ from the packed source; both independent
   # decoded goldens must still agree exactly. Keep source_exact in the manifest.

   container=webm(data,(width,height));(root/wrapped).write_bytes(container)
   records.append(dict(file=file,sha256=hashlib.sha256(data).hexdigest(),reference=expected,reference_sha256=hashlib.sha256(golden).hexdigest(),source_sha256=hashlib.sha256(pixels).hexdigest(),source_exact=source_exact,oracles=['libaom','dav1d'],frames=1,layout=layout,subsampling=[sx,sy],webm=wrapped,webm_sha256=hashlib.sha256(container).hexdigest(),depth=depth,sb=sb,lossless=lossless,orientation=orientation,size=[width,height]))
 (root/(prefix+'-generated.json')).write_text(json.dumps(dict(fixtures=records),indent=2)+'\n')
if __name__=='__main__':main()
