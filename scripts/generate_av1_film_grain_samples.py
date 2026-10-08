#!/usr/bin/env python3
"""Owned synthetic AV1 film grain fixtures, generated separately from offline tests."""
import argparse,hashlib,json,subprocess,tempfile,itertools
from pathlib import Path
from generate_av1_show_existing_samples import webm

def main():
 p=argparse.ArgumentParser(description=__doc__);p.add_argument('--tools',action='store_true');p.add_argument('--custom',action='store_true');p.add_argument('--show-existing',action='store_true');p.add_argument('--encoder',type=Path,required=True);p.add_argument('--oracle',type=Path,required=True);p.add_argument('--second-oracle',type=Path,required=True);a=p.parse_args()
 root=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors';records=[]
 prefix="av1-film-grain-show-existing" if a.show_existing else "av1-film-grain-custom" if a.custom else "av1-film-grain-tools" if a.tools else "av1-film-grain"
 frames=4 if a.tools or a.custom or a.show_existing else 1
 for depth,grain in itertools.product([8,10,12],range(8) if a.custom else range(1,17) if a.tools else [1,16]):
   width,height=(149,85) if a.tools or a.custom or a.show_existing else (64,64)
   def plane(w,h,p,t):
    tile_w,tile_h=(64,32) if p==0 else (32,16)
    values=[32+((((x+t*3)%tile_w)*17+((y+t*2)%tile_h)*31+(((x+t*3)%tile_w)*((y+t*2)%tile_h)%23)*13+p*43)%192) for y in range(h) for x in range(w)]
    if depth==8:return bytes(values)
    return b''.join((v<<(depth-8)).to_bytes(2,'little') for v in values)
   sources=[plane(width,height,0,t)+plane((width+1)//2,(height+1)//2,1,t)+plane((width+1)//2,(height+1)//2,2,t) for t in range(frames)]
   pixels=b"".join(sources)
   name=f'{prefix}-depth{depth}-lag{grain//2}-chroma{grain%2}' if a.custom else f'{prefix}-depth{depth}-preset{grain}';file=name+'.obu';expected=name+'.yuv';wrapped=name+'.webm'
   with tempfile.TemporaryDirectory(prefix='fvid-owned-grain-') as tmp:
    input=Path(tmp)/'owned.y4m';input.write_bytes(f'YUV4MPEG2 W{width} H{height} F50:1 Ip A1:1 C{"420jpeg" if depth==8 else "420p"+str(depth)}\nFRAME\n'.encode()+b'FRAME\n'.join(sources))
    grain_args=[f'--film-grain-test={grain}']
    if a.custom:
     lag=grain//2; from_luma=grain%2; n=2*lag*(lag+1)
     lines=['filmgrn1','E 0 1000000000 1 23456 1',f'p {lag} {6+lag} {grain%4} {8+lag} {from_luma} {grain%2} 153 98 275 105 149 241', 'sY 3 0 37 128 129 255 23']
     lines += ['sCb 0','sCr 0'] if from_luma else ['sCb 3 0 45 117 141 255 31','sCr 3 0 27 173 97 255 51']
     for channel in range(3):
      size=n if channel==0 else n+1
      lines.append(['cY','cCb','cCr'][channel]+' '+' '.join(str((i*17+channel*7+grain*3)%31-15) for i in range(size)))
     table=Path(tmp)/'owned-grain.tbl';table.write_text('\n'.join(lines)+'\n');grain_args=[f'--film-grain-table={table}']
    subprocess.run([str(a.encoder),'--obu','--cpu-used=0','--passes=1',f'--limit={frames}','--kf-min-dist=99','--kf-max-dist=99','--lossless=0',*grain_args,'--end-usage=q','--cq-level=32','--lag-in-frames=0','--sb-size=64',f'--bit-depth={depth}',f'--input-bit-depth={depth}','--min-partition-size=8','--max-partition-size=32','--enable-rect-partitions=0','--enable-ab-partitions=0','--enable-1to4-partitions=0','--tune-content=screen','--enable-palette=0','--enable-intrabc=0','--enable-cdef=0','--enable-restoration=0','--enable-ref-frame-mvs=0',f'--output={root/file}',str(input)],check=True)
   data=(root/file).read_bytes();subprocess.run([str(a.oracle),'--i420','--rawvideo',f'--output-bit-depth={depth}',f'--output={root/expected}',str(root/file)],check=True)
   golden=(root/expected).read_bytes()
   with tempfile.TemporaryDirectory(prefix='fvid-second-grain-oracle-') as tmp:
    cross=Path(tmp)/'cross.yuv';subprocess.run([str(a.second_oracle),'--demuxer','section5','--muxer','yuv','--filmgrain','1','-i',str(root/file),'-o',str(cross)],check=True)
    assert cross.read_bytes()==golden, name+': independent oracle mismatch'
   shown=frames;show_slot=None
   if a.show_existing:
    with tempfile.TemporaryDirectory(prefix='fvid-owned-grain-show-') as tmp:
     frame_bytes=len(golden)//frames
     for slot in range(8):
      candidate=data+bytes([0x12,0,0x1a,1,0x88+(slot<<4)])
      coded=Path(tmp)/'show.obu';coded.write_bytes(candidate)
      first=Path(tmp)/'first.yuv';second=Path(tmp)/'second.yuv'
      one=subprocess.run([str(a.oracle),'--i420','--rawvideo',f'--output-bit-depth={depth}',f'--output={first}',str(coded)],stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
      two=subprocess.run([str(a.second_oracle),'--demuxer','section5','--muxer','yuv','--filmgrain','1','-i',str(coded),'-o',str(second)],stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
      if one.returncode or two.returncode:continue
      result=first.read_bytes()
      if result!=second.read_bytes() or len(result)!=len(golden)+frame_bytes:continue
      assert result[:len(golden)]==golden
      if result[-frame_bytes:] not in [golden[i*frame_bytes:(i+1)*frame_bytes] for i in range(frames)]:continue
      data=candidate;golden=result;show_slot=slot;shown+=1;break
     assert show_slot is not None, name+': no showable grain reference'
    (root/file).write_bytes(data);(root/expected).write_bytes(golden)
   source_exact=golden==pixels

   container=webm(data,(width,height));(root/wrapped).write_bytes(container)
   records.append(dict(file=file,sha256=hashlib.sha256(data).hexdigest(),reference=expected,reference_sha256=hashlib.sha256(golden).hexdigest(),source_sha256=hashlib.sha256(pixels).hexdigest(),source_exact=source_exact,oracles=['libaom','dav1d'],frames=shown,coded_frames=frames,show_existing_slot=show_slot,subsampling='420',webm=wrapped,webm_sha256=hashlib.sha256(container).hexdigest(),depth=depth,grain_preset=None if a.custom else grain,ar_lag=grain//2 if a.custom else None,chroma_from_luma=bool(grain%2) if a.custom else None,size=[width,height]))
 (root/(prefix+'-generated.json')).write_text(json.dumps(dict(fixtures=records),indent=2)+'\n')
if __name__=='__main__':main()
