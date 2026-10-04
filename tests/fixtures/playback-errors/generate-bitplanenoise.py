"""Synthetic input only; expected results qualified separately in the reference bench."""
from pathlib import Path
p=Path(__file__).parent
out=bytearray(b'YUV4MPEG2 W16 H12 F25:1 Ip A1:1 C420jpeg\n')
for n in range(4):
 out+=b'FRAME\n'+bytes((i*37+n*23+101)%256 for i in range(288))
(p/'bitplanenoise.y4m').write_bytes(out)
