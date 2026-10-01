#!/usr/bin/env python3
"""Hand-authored two-frame Y4M/RGB patterns; FFmpeg writes oracle bytes only."""
from pathlib import Path
import subprocess
root=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors'
for name,depth,sub,tag,format,mapping in [
    ('shuffleplanes-444-8',8,1,'444','yuv444p','1:2:0:0'),
    ('shuffleplanes-420-10',10,2,'420p10','yuv420p10le','0:2:1:0'),
    ('shuffleplanes-420-10-promote',10,2,'420p10','yuv444p10le','1:2:0:0'),
    ('shuffleplanes-444-16',16,1,'444p16','yuv444p16le','2:2:0:0'),
]:
    data=bytearray(f'YUV4MPEG2 W8 H8 F25:1 Ip A1:1 C{tag}\n'.encode())
    for frame in range(2):
        data+=b'FRAME\n'
        for plane in range(3):
            n=64 if plane==0 else (8//sub)**2
            for index in range(n):
                value=(17+plane*173+frame*89+index*31)% (1<<depth)
                data+=value.to_bytes(1 if depth==8 else 2,'little')
    source=root/(name+'.y4m');source.write_bytes(data)
    subprocess.run(['ffmpeg','-v','error','-i',str(source),'-c:v','ffv1','-level','1','-coder','1','-context','0','-y',str(root/(name+'.mkv'))],check=True)
    filter=('scale=flags=neighbor,format='+format+',' if name.endswith('promote') else '')+'shuffleplanes='+mapping
    subprocess.run(['ffmpeg','-v','error','-i',str(source),'-vf',filter,'-pix_fmt',format,'-f','rawvideo','-y',str(root/(name+'.yuv'))],check=True)
raw=bytes((i*37+17)%256 for i in range(8*8*3*2));(root/'shuffleplanes-rgb.rgb').write_bytes(raw)
subprocess.run(['ffmpeg','-v','error','-f','rawvideo','-pixel_format','rgb24','-video_size','8x8','-i',str(root/'shuffleplanes-rgb.rgb'),'-vf','format=gbrp,shuffleplanes=1:2:0:0,format=rgb24','-f','rawvideo','-pix_fmt','rgb24','-y',str(root/'shuffleplanes-rgb-reference.rgb')],check=True)
