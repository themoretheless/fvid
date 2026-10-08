#!/usr/bin/env python3
"""Owned AAC ancillary extensions. Explicit generation; no external codecs."""
from pathlib import Path
import hashlib, json
root = Path(__file__).resolve().parents[1]
folder = root / 'tests/fixtures/playback-errors'
source = (folder / 'aac-independent-coupling.aac').read_bytes()
cases = {
    'empty': bytes([0x20, 0]),
    'short': bytes([0x20, 3]) + b'FVi',
    'escaped': bytes([0x20, 255, 0]) + bytes(range(255)),
    'multiple': bytes([0x20, 3]) + b'FVi' + bytes([0x20, 2]) + b'id' + bytes([0x10, 0xa5, 0xa5]),
    'overrun': bytes([0x20, 7, 0]),
    'unterminated': bytes([0x20, 255]),
    'version': bytes([0x21, 0]),
    'hidden-sbr': bytes([0x20, 0, 0xd0]),
    'bad-fill': bytes([0x20, 0, 0x10, 0xa4]),
}
manifest = {'source_sha256': hashlib.sha256(source).hexdigest(), 'cases': {}}
for name, extension in cases.items():
    n = len(extension)
    bits = '110' + (f'{n:04b}' if n < 15 else '1111' + f'{n-14:08b}')
    bits += ''.join(f'{byte:08b}' for byte in extension) + '1100000' * 7
    assert len(bits) % 8 == 0
    prefix = int(bits, 2).to_bytes(len(bits)//8, 'big')
    output = bytearray(); at = 0
    while at < len(source):
        size = ((source[at+3] & 3)<<11) | (source[at+4]<<3) | (source[at+5]>>5)
        payload = prefix + source[at+7:at+size]
        length = len(payload)+7
        header = bytearray(source[at:at+7])
        header[3] = (header[3]&252) | (length>>11)
        header[4] = (length>>3)&255
        header[5] = (header[5]&31) | ((length&7)<<5)
        output += header + payload; at += size
    path = folder / f'aac-fill-{name}.aac'; path.write_bytes(output)
    manifest['cases'][name] = {'file': path.name, 'sha256': hashlib.sha256(output).hexdigest()}
video = b'YUV4MPEG2 W16 H16 F48000:1024 Ip A1:1 C420jpeg\n'
for frame in range(6):
    video += b'FRAME\n' + bytes(32+(x+y+frame*3)%160 for y in range(16) for x in range(16)) + bytes([128])*128
path = folder/'aac-fill-companion.y4m';path.write_bytes(video)
manifest['video'] = {'file':path.name, 'sha256':hashlib.sha256(video).hexdigest()}
(folder/'aac-fill-generated.json').write_text(json.dumps(manifest,indent=2)+'\n')
