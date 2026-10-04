"""Synthetic input only; qualification is separate from test execution."""
from pathlib import Path
p=Path(__file__).parent
for name,w,h,size,extra in [('lenscorrection',16,12,288,''),('lenscorrection-full',16,12,288,' XCOLORRANGE=FULL'),('lenscorrection-small',1,1,3,'')]:
 data=bytearray(f'YUV4MPEG2 W{w} H{h} F25:1 Ip A1:1 C420jpeg{extra}\n'.encode())
 for n in range(4):data+=b'FRAME\n'+bytes((i*37+n*23+101)%256 for i in range(size))
 (p/f'{name}.y4m').write_bytes(data)
