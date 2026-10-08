#!/usr/bin/env python3
"""Owned AV1 super-resolution + inherited segmentation fixture generation.

For libaom v3.15.1, apply av1_superres_variance_aq_fixture.patch before
building aomenc. This generation-only patch prevents variance AQ from being
suppressed by the encoder's resolution-change heuristic for super-resolution.
It also selects LAST as primary reference and forces stable inter-frame maps.
Use this patched encoder only for these owned fixed-size fixtures.
Validate saved pixels using unmodified libaom and independent dav1d decoders.
Ordinary tests use saved assets and do not invoke any external codecs.
"""
import argparse,hashlib,json,subprocess,tempfile,itertools
from pathlib import Path
from generate_av1_show_existing_samples import webm

def main():
 p=argparse.ArgumentParser(description=__doc__);p.add_argument('--case-limit',type=int);p.add_argument('--encoder',type=Path,required=True);p.add_argument('--oracle',type=Path,required=True);p.add_argument('--second-oracle',type=Path,required=True);a=p.parse_args();prefix='av1-superres-segmentation'
 root=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors';records=[]
 cases=list(itertools.product([8,10,12],[3],[True],[1],[True],[32],range(2),[9,12,16]))
 for depth,colors,chroma,layout,mirror,quality,orientation,denominator in cases[:a.case_limit]:
   width,height=192,128
   def plane(w,h,p,t):
    values=[max(0,min(255,64+((x+t*2)%w)*90//w+y*60//h+(((x*17+y*31+p*11+t*3)%17)-8)*(2 if orientation==0 else 5))) for y in range(h) for x in range(w)]
    if depth==8:return bytes(values)
    return b''.join((v<<(depth-8)).to_bytes(2,'little') for v in values)
   pixels=b"".join(plane(width,height,0,t)+plane((width+1)//2,(height+1)//2,1,t)+plane((width+1)//2,(height+1)//2,2,t) for t in range(3))
   name=f'{prefix}-depth{depth}-colors{colors}-chroma{int(chroma)}-layout{layout}-vflip{int(mirror)}-q{quality}-orientation{orientation}-denom{denominator}';file=name+'.obu';expected=name+'.yuv';wrapped=name+'.webm'
   with tempfile.TemporaryDirectory(prefix='fvid-owned-palette-') as tmp:
    input=Path(tmp)/'owned.y4m';input.write_bytes(f'YUV4MPEG2 W{width} H{height} F50:1 Ip A1:1 C{"420jpeg" if depth==8 else "420p"+str(depth)}\nFRAME\n'.encode()+b'FRAME\n'.join(pixels[i:i+len(pixels)//3] for i in range(0,len(pixels),len(pixels)//3)))
    subprocess.run([str(a.encoder),'--obu','--cpu-used=0','--passes=2','--sb-size=64','--limit=3','--lag-in-frames=0','--auto-alt-ref=0','--kf-min-dist=99','--kf-max-dist=99','--lossless=0','--end-usage=q',f'--cq-level={quality}','--deltaq-mode=0','--loopfilter-control=1','--enable-qm=0','--aq-mode=1','--superres-mode=1',f'--superres-denominator={denominator}',f'--superres-kf-denominator={denominator}',f'--bit-depth={depth}',f'--input-bit-depth={depth}','--min-partition-size=4','--max-partition-size=64','--enable-rect-partitions=0','--enable-ab-partitions=0','--enable-1to4-partitions=0','--tune-content=default','--enable-palette=0','--enable-intrabc=0','--enable-cdef=1','--enable-restoration=1','--enable-ref-frame-mvs=0',f'--output={root/file}',str(input)],check=True)
   data=(root/file).read_bytes();subprocess.run([str(a.oracle),str(root/file),'3',str(root/expected)],check=True)
   golden=(root/expected).read_bytes()
   with tempfile.TemporaryDirectory(prefix='fvid-second-palette-oracle-') as tmp:
    cross=Path(tmp)/'cross.yuv';subprocess.run([str(a.second_oracle),str(root/file),str(cross),"3"],check=True)
    assert cross.read_bytes()==golden, name+': independent oracle mismatch'
   source_exact=golden==pixels


   container=webm(data,(width,height));(root/wrapped).write_bytes(container)
   records.append(dict(file=file,sha256=hashlib.sha256(data).hexdigest(),reference=expected,reference_sha256=hashlib.sha256(golden).hexdigest(),source_sha256=hashlib.sha256(pixels).hexdigest(),source_exact=source_exact,quality=quality,orientation=orientation,denominator=denominator,oracles=['libaom','dav1d'],mirror=mirror,webm=wrapped,webm_sha256=hashlib.sha256(container).hexdigest(),depth=depth,colors=colors,chroma=chroma,layout=layout,size=[width,height]))
 (root/(prefix+'-generated.json')).write_text(json.dumps(dict(fixtures=records, encoder_source='libaom v3.15.1', encoder_fixture_patch='scripts/av1_superres_variance_aq_fixture.patch', encoder_fixture_patch_sha256=hashlib.sha256(Path(__file__).with_name('av1_superres_variance_aq_fixture.patch').read_bytes()).hexdigest()),indent=2)+'\n')
if __name__=='__main__':main()
