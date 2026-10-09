#!/usr/bin/env python3
"""Own future-extension tails on authored HEVC parameter sets; offline."""
import struct
from generate_he_aac_packet_fixtures import DEST, boxes
from hevc_fixture_mp4 import box


def extended(nal, value, tail, bad_stop=False, raised_level=False):
    rbsp = bytearray()
    zeros = 0
    for byte in nal[2:]:
        if zeros == 2 and byte == 3:
            zeros = 0
            continue
        rbsp.append(byte)
        zeros = zeros + 1 if byte == 0 else 0
    if raised_level:
        assert (nal[0] >> 1) & 63 == 32 and rbsp[1] & 15 == 1
        assert rbsp[15] == 30
        rbsp[15] = 60
    bits = ''.join(f'{b:08b}' for b in rbsp)
    stop = bits.rfind('1')
    syntax = bits[:stop]
    assert syntax[-1] == '0', 'seed must have extension_present_flag zero'
    flags = '' if (nal[0] >> 1) & 63 == 32 else '0000' + f'{value:04b}'
    bits = syntax[:-1] + '1' + flags + tail + ('0' if bad_stop else '1')
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
        ('vps', {32}, 0, '001010000000000000000000000000001110'),
        ('vps-bad-stop', {32}, 0, '00000000000000000000000000000000'),
        ('vps-wrong-id', {32}, 0, ''),
        ('paired-id', {32, 33}, 0, ''),
        ('vps-level-change', {32}, 0, '00101'),
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
                        if label in ('vps-wrong-id', 'paired-id'):
                            nal = nal[:2] + bytes([(nal[2] & 15) | 16]) + nal[3:]
                        else:
                            nal = extended(nal, value, tail, label.endswith('bad-stop'), label == 'vps-level-change')
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


    def child(data, tag):
        return next(p for t, p in boxes(data) if t == tag)
    root = dict(roots)
    stbl = child(child(child(child(root[b'moov'], b'trak'), b'mdia'), b'minf'), b'stbl')
    sizes = child(stbl, b'stsz')
    constant, count = struct.unpack_from('>II', sizes, 4)
    assert constant == 0
    lengths = struct.unpack('>' + str(count) + 'I', sizes[12:])
    chunks = child(stbl, b'stco')
    assert int.from_bytes(chunks[4:8], 'big') == 1
    at = int.from_bytes(chunks[8:12], 'big')
    packets = []
    for n in lengths:
        packets.append(seed[at:at+n])
        at += n
    entry = next(boxes(child(stbl, b'stsd')[8:]))[1]
    hvcc = child(entry[78:], b'hvcC')
    assert hvcc[23] & 63 == 32 and int.from_bytes(hvcc[24:26], 'big') == 1
    size = int.from_bytes(hvcc[26:28], 'big')
    nal = extended(hvcc[28:28+size], 0, '00101', raised_level=True)
    packets[1] = struct.pack('>I', len(nal)) + nal + packets[1]
    def inter_switch(tag, data):
        if tag in (b'moov', b'trak', b'mdia', b'minf', b'stbl'):
            data = b''.join(inter_switch(t, p) for t, p in boxes(data))
        elif tag == b'stsz':
            data = data[:12] + b''.join(struct.pack('>I', len(p)) for p in packets)
        elif tag == b'mdat':
            data = b''.join(packets)
        return box(tag, data)
    (DEST / 'hevc-vps-change-inter-synthetic.mp4').write_bytes(b''.join(inter_switch(t, p) for t, p in roots))


if __name__ == '__main__':
    main()
