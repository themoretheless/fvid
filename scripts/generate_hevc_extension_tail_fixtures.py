#!/usr/bin/env python3
"""Own future-extension tails on authored HEVC parameter sets; offline."""
import struct
from generate_he_aac_packet_fixtures import DEST, boxes
from hevc_fixture_mp4 import box


def extended(nal, value, tail, bad_stop=False):
    rbsp = bytearray()
    zeros = 0
    for byte in nal[2:]:
        if zeros == 2 and byte == 3:
            zeros = 0
            continue
        rbsp.append(byte)
        zeros = zeros + 1 if byte == 0 else 0
    bits = ''.join(f'{b:08b}' for b in rbsp)
    stop = bits.rfind('1')
    syntax = bits[:stop]
    assert syntax[-1] == '0', 'seed must have extension_present_flag zero'
    bits = syntax[:-1] + '1' + '0000' + f'{value:04b}' + tail + ('0' if bad_stop else '1')
    bits += '0' * (-len(bits) % 8)
    raw = bytes(int(bits[i:i+8], 2) for i in range(0, len(bits), 8))
    escaped = bytearray(nal[:2])
    zeros = 0
    for byte in raw:
        if zeros == 2 and byte <= 3:
            escaped.append(3)
            zeros = 0
        escaped.append(byte)
        zeros = zeros + 1 if byte == 0 else 0
    return bytes(escaped)


def main():
    seed = (DEST.parent / 'hevc' / 'main-ipb.mp4').read_bytes()
    roots = list(boxes(seed))
    assert roots[-1][0] == b'moov', 'metadata must follow media offsets'
    for label, kinds, value, tail in [
        ('sps', {33}, 1, '001001000000000000000000000000001'),
        ('pps', {34}, 15, '00000000000000000000000000000000'),
        ('both', {33, 34}, 9, '101010111001000101'),
        ('bad-stop', {33}, 1, '00000000000000000000000000000000'),
    ]:
        def config(data):
            result = bytearray(data[:23])
            at = 23
            for _ in range(data[22]):
                header = data[at:at+3]
                at += 3
                result += header
                for _ in range(int.from_bytes(header[1:], 'big')):
                    n = int.from_bytes(data[at:at+2], 'big')
                    at += 2
                    nal = data[at:at+n]
                    at += n
                    if header[0] & 63 in kinds:
                        nal = extended(nal, value, tail, label == 'bad-stop')
                    result += struct.pack('>H', len(nal)) + nal
            assert at == len(data)
            return bytes(result)
        def rewrite(tag, data):
            if tag in (b'moov', b'trak', b'mdia', b'minf', b'stbl'):
                data = b''.join(rewrite(t, p) for t, p in boxes(data))
            elif tag == b'stsd':
                data = data[:8] + b''.join(box(t, p[:78] + b''.join(box(k, config(v) if k == b'hvcC' else v) for k, v in boxes(p[78:]))) for t, p in boxes(data[8:]))
            return box(tag, data)
        movie = b''.join(rewrite(t, p) for t, p in roots)
        (DEST / f'hevc-future-extension-{label}-synthetic.mp4').write_bytes(movie)


if __name__ == '__main__':
    main()
