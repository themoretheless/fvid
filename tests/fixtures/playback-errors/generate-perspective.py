"""Four synthetic coordinate ramps; independent of FFmpeg and private media."""
from pathlib import Path
root = Path(__file__).parent
for name, width, height, sampling, depth in [
    ('perspective', 17, 13, 2, 8),
    ('perspective-depth', 17, 13, 1, 16),
    ('perspective-small', 1, 1, 2, 8),
]:
    chroma = '420jpeg' if sampling == 2 else '444p16'
    output = bytearray(f'YUV4MPEG2 W{width} H{height} F25:1 Ip A1:1 C{chroma}\n'.encode())
    for n in range(4):
        output += b'FRAME\n'
        for plane in range(3):
            w, h = (width, height) if plane == 0 else ((width+sampling-1)//sampling, (height+sampling-1)//sampling)
            for y in range(h):
                for x in range(w):
                    value = (((x*7+y*11+plane*53+n*13)%192)+32) << (depth-8)
                    output += value.to_bytes(1 if depth == 8 else 2, 'little')
    (root / f'{name}.y4m').write_bytes(output)
