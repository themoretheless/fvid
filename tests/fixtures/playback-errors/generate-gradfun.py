"""Synthetic only. Qualification/goldens belong to the explicit reference benchmark."""
from pathlib import Path
p=Path(__file__).parent
for name,w,h,tag,size in [('gradfun',96,80,'420jpeg',11520),('gradfun-edge',65,65,'444',12675),('gradfun-overflow',9,9,'444',243),('gradfun-small',1,1,'420jpeg',3)]:
 out=bytearray(f'YUV4MPEG2 W{w} H{h} F25:1 Ip A1:1 C{tag}\n'.encode())
 for n in range(4):out+=b'FRAME\n'+bytes((i*37+n*23+101)%256 for i in range(size))
 (p/f'{name}.y4m').write_bytes(out)
