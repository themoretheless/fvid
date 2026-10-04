"""Synthetic box/grid input only; reference generation is a separate explicit benchmark."""
from pathlib import Path
p=Path(__file__).parent
for name,w,h,aspect in [('drawing',17,13,'1:1'),('drawing-aspect',17,13,'4:3'),('drawing-small',1,1,'1:1')]:
 size=w*h+2*((w+1)//2)*((h+1)//2)
 out=bytearray(f'YUV4MPEG2 W{w} H{h} F25:1 Ip A{aspect} C420jpeg\n'.encode())
 for n in range(4):out+=b'FRAME\n'+bytes((i*37+n*23+101)%256 for i in range(size))
 (p/f'{name}.y4m').write_bytes(out)
