#!/usr/bin/env python3
"""Owned synthetic AV1 4:2:2/4:4:4 chroma streams; generation only."""
import argparse, hashlib, itertools, json, subprocess, tempfile
from pathlib import Path
from generate_av1_show_existing_samples import webm

def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--encoder',type=Path,required=True)
    parser.add_argument('--oracle',type=Path,required=True)
    parser.add_argument('--second-oracle',type=Path,required=True)
    args=parser.parse_args()
    root=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors'
    records=[]
    for depth,layout,lossless in itertools.product([8,10,12],["422","444"],[True,False]):
        orientation=0
        w,h=64,48;frames=2
        def plane(width,height,channel,t):
            values=[]
            for y in range(height):
                for x in range(width):
                    xx,yy=(y,x) if orientation==2 else (x,y)
                    # Most cells translate predictably; sparse 4-sample patches
                    # change each frame to force intra neighbors in shared UV groups.
                    moving=((xx+t*3)*17+(yy+t*2)*29+channel*47)%137
                    fresh=((xx//4+yy//4+t)%7==0)
                    mix=((xx+1)*0x45d9f3b)^((yy+3)*0x27d4eb2d)^((t+5)*0x165667b1)^channel*239
                    mix=((mix^(mix>>13))*0x85ebca6b)&0xffffffff
                    value=40+((mix^(mix>>16))%176 if fresh else moving)
                    if orientation: value=48+((xx*7+(yy//4)*29+t*13+channel*43)%128) if not fresh else value
                    values.append(value)
            return bytes(values) if depth==8 else b''.join((v<<(depth-8)).to_bytes(2,'little') for v in values)
        sx,sy=(1,0) if layout=="422" else (0,0)
        source=[plane(w,h,0,t)+plane(w>>sx,h>>sy,1,t)+plane(w>>sx,h>>sy,2,t) for t in range(frames)]
        stem=f'av1-chroma-layout{layout}-depth{depth}-lossless{int(lossless)}'
        coded=root/(stem+'.obu');golden=root/(stem+'.yuv');container=root/(stem+'.webm')
        with tempfile.TemporaryDirectory(prefix='fvid-owned-chroma-') as tmp:
            input=Path(tmp)/'owned.y4m'
            input.write_bytes(f'YUV4MPEG2 W{w} H{h} F50:1 Ip A1:1 C{layout if depth==8 else layout+"p"+str(depth)}\nFRAME\n'.encode()+b'FRAME\n'.join(source))
            # At 12 bits let aomenc infer profile 2 from the Y4M input; forcing
            # it enters the CLI's chroma-control path before encoder creation.
            subprocess.run([str(args.encoder),'--obu','--cpu-used=0','--passes=1',f'--limit={frames}','--sb-size=64','--lag-in-frames=0','--auto-alt-ref=0','--kf-min-dist=99','--kf-max-dist=99',f'--lossless={int(lossless)}',*([f'--profile={1 if layout=="444" else 2}'] if depth<12 else []),'--end-usage=q','--cq-level=16','--deltaq-mode=0','--loopfilter-control=0','--enable-qm=0',f'--bit-depth={depth}',f'--input-bit-depth={depth}','--min-partition-size=4','--max-partition-size=32',f'--enable-rect-partitions={int(orientation>0)}',f'--enable-ab-partitions={int(orientation>0)}',f'--enable-1to4-partitions={int(orientation>0)}','--tune-content=default','--enable-palette=0','--enable-intrabc=0','--enable-cdef=0','--enable-restoration=0','--enable-ref-frame-mvs=0',f'--output={coded}',str(input)],check=True)
            subprocess.run([str(args.oracle),'--i420','--rawvideo',f'--output-bit-depth={depth}',f'--output={golden}',str(coded)],check=True)
            second=Path(tmp)/'second.yuv'
            subprocess.run([str(args.second_oracle),'--demuxer','section5','--muxer','yuv','--filmgrain','1','-i',str(coded),'-o',str(second)],check=True)
            assert second.read_bytes()==golden.read_bytes(),stem+': independent oracle mismatch'
            if lossless:assert golden.read_bytes()==b''.join(source),stem+': lossless source mismatch'
        container.write_bytes(webm(coded.read_bytes(),(w,h)))
        records.append(dict(file=coded.name,reference=golden.name,webm=container.name,sha256=hashlib.sha256(coded.read_bytes()).hexdigest(),reference_sha256=hashlib.sha256(golden.read_bytes()).hexdigest(),webm_sha256=hashlib.sha256(container.read_bytes()).hexdigest(),source_sha256=hashlib.sha256(b''.join(source)).hexdigest(),depth=depth,layout=layout,lossless=lossless,quality=16,subsampling=[sx,sy],frames=frames,size=[w,h],oracles=['libaom','dav1d']))
    (root/'av1-chroma-generated.json').write_text(json.dumps(dict(fixtures=records),indent=2)+'\n')
if __name__=='__main__':main()
