#!/usr/bin/env python3
"""Own integer Y4M/RGB patterns, analytical shuffle oracle and FVid FFV1; no FFmpeg.

Build: cargo build --no-default-features --bin fvid
Generate: python3 scripts/generate_shuffleplanes_samples.py --fvid target/debug/fvid
Optional reference benchmark: python3 scripts/benchmark_shuffleplanes_reference.py
Ordinary tests use checked-in bytes and invoke neither generator nor reference tool.
"""
import argparse
import json
from pathlib import Path
import subprocess
import tempfile

parser=argparse.ArgumentParser(description=__doc__)
parser.add_argument('--fvid',type=Path,required=True,help='FVid built with --no-default-features')
parser.add_argument('--output',type=Path,default=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors')
args=parser.parse_args()
root=args.output
root.mkdir(parents=True,exist_ok=True)
with tempfile.TemporaryDirectory(prefix='fvid-shuffle-fixtures-') as temp:
    for name,depth,sub,tag,mapping,promote in [
        ('shuffleplanes-444-8',8,1,'444',[1,2,0],False),
        ('shuffleplanes-420-10',10,2,'420p10',[0,2,1],False),
        ('shuffleplanes-420-10-promote',10,2,'420p10',[1,2,0],True),
        ('shuffleplanes-444-16',16,1,'444p16',[2,2,0],False),
    ]:
        data=bytearray(f'YUV4MPEG2 W8 H8 F25:1 Ip A1:1 C{tag}\n'.encode())
        oracle=bytearray()
        for frame in range(2):
            data+=b'FRAME\n'
            planes=[]
            for plane in range(3):
                side=8 if plane==0 else 8//sub
                values=[(17+plane*173+frame*89+index*31)%(1<<depth) for index in range(side*side)]
                planes.append(values)
                for value in values:
                    data+=value.to_bytes(1 if depth==8 else 2,'little')
            for output_plane,input_plane in enumerate(mapping):
                output_side=8 if promote or output_plane==0 else 8//sub
                input_side=8 if input_plane==0 else 8//sub
                for y in range(output_side):
                    for x in range(output_side):
                        # Exact nearest-neighbour replication for the 420->444
                        # case; otherwise every mapped plane has matching size.
                        value=planes[input_plane][(y*input_side//output_side)*input_side+x*input_side//output_side]
                        oracle+=value.to_bytes(1 if depth==8 else 2,'little')
        source=root/(name+'.y4m')
        source.write_bytes(data)
        encoded=Path(temp)/(name+'.mkv')
        result=subprocess.run([str(args.fvid.resolve()),'media','transcode-lossless',str(source.resolve()),str(encoded)],check=True,stdout=subprocess.PIPE)
        stats=json.loads(result.stdout)
        assert stats['backend']=='fvid' and stats['encoder']=='ffv1' and stats['video_frames']==2
        (root/(name+'.mkv')).write_bytes(encoded.read_bytes())
        (root/(name+'.yuv')).write_bytes(oracle)
raw=bytes((i*37+17)%256 for i in range(8*8*3*2))
(root/'shuffleplanes-rgb.rgb').write_bytes(raw)
# GBR planar map 1:2:0 becomes packed RGB [G,B,R].
reference=bytes(value for i in range(0,len(raw),3) for value in [raw[i+1],raw[i+2],raw[i]])
(root/'shuffleplanes-rgb-reference.rgb').write_bytes(reference)
