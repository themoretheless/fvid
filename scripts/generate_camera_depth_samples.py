#!/usr/bin/env python3
"""Own high-depth Y4M patterns and exact rational RGB oracles; no external tools."""
from pathlib import Path
root=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors'
for depth,layout,sub in [(10,'420',2),(16,'444',1)]:
    name=f'camera-y4m-{depth}'
    source=root/(name+'.y4m')
    data=bytearray(f'YUV4MPEG2 W8 H8 F25:1 Ip A1:1 C{layout}p{depth} XCOLORRANGE=LIMITED\n'.encode())
    for frame in range(2):
        data+=b'FRAME\n'
        for plane in range(3):
            count=64 if plane==0 else (8//sub)**2
            for index in range(count):
                value=(16+(index*13+frame*97)%220) if plane==0 else (100+(index*4+frame*30)%50) if plane==1 else (70+(index*7+frame*60)%130)
                data+=(value<<(depth-8)).to_bytes(2,'little')
    source.write_bytes(data)
    # Independent integer/rational inverse matrix for the exact 8-bit codes
    # used by these high-depth patterns; no floating-point decoder implementation.
    def rounded(n,d):
        return 0 if n<=0 else 255 if n>=255*d else (2*n+d)//(2*d)
    rgb=bytearray()
    for frame in range(2):
        for y in range(8):
            for x in range(8):
                index=y*8+x;chroma=(y//sub)*(8//sub)+x//sub
                yy=(16+(index*13+frame*97)%220)-16
                cb=(100+(chroma*4+frame*30)%50)-128
                cr=(70+(chroma*7+frame*60)%130)-128
                rgb+=bytes([
                    rounded(yy*255*224*1000+cr*255*219*1402,219*224*1000),
                    rounded(yy*255*224*1000*587-cr*255*219*299*1402-cb*255*219*114*1772,219*224*1000*587),
                    rounded(yy*255*224*1000+cb*255*219*1772,219*224*1000),
                ])
    (root/(name+'-analytic.rgb')).write_bytes(rgb)
