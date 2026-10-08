#!/usr/bin/env python3
"""Owned synthetic AV1 odd 4:2:2/4:4:4 super-resolution chroma streams; generation only."""
import argparse, hashlib, itertools, json, subprocess, tempfile
from pathlib import Path
from generate_av1_show_existing_samples import webm

def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--encoder',type=Path,required=True,help='generation-only libaom build with fixture-tools/av1-chroma-forced-deblock.patch')
    parser.add_argument('--oracle',type=Path,required=True)
    parser.add_argument('--second-oracle',type=Path,required=True)
    args=parser.parse_args()
    root=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors'
    records=[]
    for depth,layout,(denominator,key_denominator),pattern in itertools.product([8,10,12],["422","444"],[(9,9),(12,12),(9,16)],range(2)):
        quality=32
        lossless=False
        encoder=args.encoder
        w,h=191,127;frames=3
        def plane(width,height,channel,t):
            values=[]
            for y in range(height):
                for x in range(width):
                    xx,yy=x,y
                    value=max(0,min(255,64+(xx*90//width)+(yy*60//height)+
                        (((xx*17+yy*31+channel*11+t*3)%17)-8)*(2 if pattern==0 else 5)))
                    values.append(value)
            return bytes(values) if depth==8 else b''.join((v<<(depth-8)).to_bytes(2,'little') for v in values)
        sx,sy=(1,0) if layout=="422" else (0,0)
        source=[plane(w,h,0,t)+plane((w+(1<<sx)-1)>>sx,(h+(1<<sy)-1)>>sy,1,t)+plane((w+(1<<sx)-1)>>sx,(h+(1<<sy)-1)>>sy,2,t) for t in range(frames)]
        stem=f'av1-chroma-superres-layout{layout}-depth{depth}-den{denominator}-kf{key_denominator}-pattern{pattern}'
        coded=root/(stem+'.obu');golden=root/(stem+'.yuv');container=root/(stem+'.webm')
        with tempfile.TemporaryDirectory(prefix='fvid-owned-chroma-') as tmp:
            input=Path(tmp)/'owned.y4m'
            input.write_bytes(f'YUV4MPEG2 W{w} H{h} F50:1 Ip A1:1 C{layout if depth==8 else layout+"p"+str(depth)}\nFRAME\n'.encode()+b'FRAME\n'.join(source))
            # At 12 bits let aomenc infer profile 2 from the Y4M input; forcing
            # it enters the CLI's chroma-control path before encoder creation.
            subprocess.run([str(encoder),'--obu','--cpu-used=0','--passes=1',f'--limit={frames}','--sb-size=64','--lag-in-frames=0','--auto-alt-ref=0','--kf-min-dist=99','--kf-max-dist=99',f'--lossless={int(lossless)}',*([f'--profile={1 if layout=="444" else 2}'] if depth<12 else []),'--end-usage=q',f'--cq-level={quality}','--deltaq-mode=0','--loopfilter-control=1','--enable-qm=0','--superres-mode=1',f'--superres-denominator={denominator}',f'--superres-kf-denominator={key_denominator}',f'--bit-depth={depth}',f'--input-bit-depth={depth}','--min-partition-size=4','--max-partition-size=64','--enable-rect-partitions=1','--enable-ab-partitions=0','--enable-1to4-partitions=0','--tune-content=default','--enable-palette=0','--enable-intrabc=0','--enable-cdef=1','--enable-restoration=1','--enable-ref-frame-mvs=0',f'--output={coded}',str(input)],check=True)
            subprocess.run([str(args.oracle),'--i420','--rawvideo',f'--output-bit-depth={depth}',f'--output={golden}',str(coded)],check=True)
            second=Path(tmp)/'second.yuv'
            subprocess.run([str(args.second_oracle),'--demuxer','section5','--muxer','yuv','--filmgrain','1','-i',str(coded),'-o',str(second)],check=True)
            assert second.read_bytes()==golden.read_bytes(),stem+': independent oracle mismatch'
        container.write_bytes(webm(coded.read_bytes(),(w,h)))
        records.append(dict(file=coded.name,reference=golden.name,webm=container.name,sha256=hashlib.sha256(coded.read_bytes()).hexdigest(),reference_sha256=hashlib.sha256(golden.read_bytes()).hexdigest(),webm_sha256=hashlib.sha256(container.read_bytes()).hexdigest(),source_sha256=hashlib.sha256(b''.join(source)).hexdigest(),depth=depth,layout=layout,lossless=lossless,quality=quality,pattern=pattern,subsampling=[sx,sy],frames=frames,size=[w,h],oracles=['libaom','dav1d'],denominator=denominator,key_denominator=key_denominator,cpu_used=0,forced_deblocking=True,encoder_source_commit='44d0a57786f432d933ff64b653347c66f4d0fa1d',encoder_patch_sha256=hashlib.sha256((Path(__file__).parent/'fixture-tools/av1-chroma-forced-deblock.patch').read_bytes()).hexdigest()))
    (root/'av1-chroma-superres-generated.json').write_text(json.dumps(dict(fixtures=records),indent=2)+'\n')
if __name__=='__main__':main()
