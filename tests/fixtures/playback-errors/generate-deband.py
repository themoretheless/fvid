"""Deterministic synthetic ramps; no external executables or private media."""
from pathlib import Path
p=Path(__file__).parent
for name,w,h,sub,depth in [('deband',17,13,2,8),('deband-coupled',17,13,1,8),('deband-depth',17,13,1,16),('deband-small',1,1,2,8)]:
 base='420jpeg' if sub==2 else ('444' if depth==8 else '444p16')
 out=bytearray(f'YUV4MPEG2 W{w} H{h} F25:1 Ip A1:1 C{base}\n'.encode())
 dims=[(w,h),((w+sub-1)//sub,(h+sub-1)//sub),((w+sub-1)//sub,(h+sub-1)//sub)]
 for n in range(4):
  out+=b'FRAME\n'
  for plane,(pw,ph) in enumerate(dims):
   for y in range(ph):
    for x in range(pw):
     value=((x*7+y*11+plane*53+n*13)%192+32)<< (depth-8)
     out+=value.to_bytes(1 if depth==8 else 2,'little')
 (p/f'{name}.y4m').write_bytes(out)
