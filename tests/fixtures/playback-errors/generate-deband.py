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

# Independent portable pixel oracle. All arithmetic boundary casts are explicit;
# transcendental operations are evaluated in binary64 before binary32 rounding.
# Old deband-*.expected.raw files retain the historical macOS libm reference.
import math
import struct

def f32(value):
    return struct.unpack('<f', struct.pack('<f', value))[0]

def offset(x, y):
    angle = f32(x * f32(12.9898) + f32(y * f32(78.233)))
    random = f32(f32(math.sin(angle)) * f32(43758.545))
    random = f32(random - math.trunc(random))
    if random < 0:
        random = f32(random + 1)
    direction = f32(random * f32(math.tau))
    distance = math.trunc(f32(random * 16))
    return (math.trunc(f32(f32(math.cos(direction)) * distance)),
            math.trunc(f32(f32(math.sin(direction)) * distance)))

for name, sub, depth, threshold, blur, coupled in [
    ('default', 2, 8, .02, True, False),
    ('strong', 2, 8, .5, True, False),
    ('no-blur', 2, 8, .5, False, False),
    ('coupled', 1, 8, .5, True, True),
    ('depth', 1, 16, .5, True, True),
]:
    out = bytearray()
    limit = math.trunc(f32(((1 << depth) - 1) * f32(threshold)))
    dims = [(17, 13), ((17 + sub - 1) // sub, (13 + sub - 1) // sub)]
    dims.append(dims[1])
    for n in range(4):
        results = []
        for plane, (w, h) in enumerate(dims):
            values = []
            for y in range(h):
                for x in range(w):
                    dx, dy = offset(x, y)
                    def sample(a, b):
                        xx, yy = max(0, min(w - 1, a)), max(0, min(h - 1, b))
                        return (((xx * 7 + yy * 11 + plane * 53 + n * 13) % 192) + 32) << (depth - 8)
                    src = sample(x, y)
                    refs = [sample(x+a, y+b) for a, b in [(dx,dy),(dx,-dy),(-dx,-dy),(-dx,dy)]]
                    avg = sum(refs) // 4
                    passes = abs(src - avg) < limit if blur else all(abs(src-v) < limit for v in refs)
                    values.append((src, avg, passes))
            results.append(values)
        for values in results:
            for i, (src, avg, passes) in enumerate(values):
                if coupled:
                    passes = all(plane[i][2] for plane in results)
                out += (avg if passes else src).to_bytes(1 if depth == 8 else 2, 'little')
    (p / f'deband-portable-{name}.expected.raw').write_bytes(out)
