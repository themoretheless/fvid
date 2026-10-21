"""Owned HEVC explicit zero CLL across independent sequences; offline."""
from copy import deepcopy
from generate_hevc_hdr_seek import HDR, ROOT, generate as generate_seek
OUT = ROOT / 'hevc-hdr-zero'
def generate():
    values = deepcopy(HDR)
    values[1].update(cll=0, fall=0)
    return generate_seek(values)
if __name__ == '__main__':
    OUT.mkdir(exist_ok=True)
    for name, data in generate().items():
        (OUT / name).write_bytes(data)
