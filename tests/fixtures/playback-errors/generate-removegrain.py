"""Short synthetic grain input; no codecs, external programs or private pixels."""
from pathlib import Path
p=Path(__file__).parent
for name,w,h in [('removegrain',17,13),('removegrain-small',1,1)]:
 size=w*h+2*((w+1)//2)*((h+1)//2)
 out=bytearray(f'YUV4MPEG2 W{w} H{h} F25:1 Ip A1:1 C420jpeg\n'.encode())
 for n in range(4):
  out+=b'FRAME\n'+bytes((i*i*13+i*37+n*23+101)%256 for i in range(size))
 (p/f'{name}.y4m').write_bytes(out)
