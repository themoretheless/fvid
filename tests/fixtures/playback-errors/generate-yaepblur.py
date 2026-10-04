"""Synthetic inputs only. Expected outputs are qualified in explicit reference benches."""
from pathlib import Path
p=Path(__file__).parent
for name,w,h,extra in [('yaepblur',16,12,''),('yaepblur-full',16,12,' XCOLORRANGE=FULL'),('yaepblur-small',1,1,'')]:
 size=w*h+2*((w+1)//2)*((h+1)//2)
 data=bytearray(f'YUV4MPEG2 W{w} H{h} F25:1 Ip A1:1 C420jpeg{extra}\n'.encode())
 for n in range(4):data+=b'FRAME\n'+bytes((i*37+n*23+101)%256 for i in range(size))
 (p/f'{name}.y4m').write_bytes(data)
w=257
out=bytearray(f'YUV4MPEG2 W{w} H{w} F25:1 Ip A1:1 C444p16\nFRAME\n'.encode())
for plane in range(3):
 for i in range(w*w):
  value=(0 if i==(w*w)//2 else 65535) if plane==0 else 32768
  out+=value.to_bytes(2,'little')
(p/'yaepblur-wide.y4m').write_bytes(out)
