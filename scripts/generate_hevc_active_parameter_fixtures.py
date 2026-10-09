#!/usr/bin/env python3
"""Authored active-parameter SEI on own short IPB source, offline."""
import struct
from generate_he_aac_packet_fixtures import DEST, boxes
from hevc_fixture_mp4 import box


def ue(v):
    b = bin(v + 1)[2:]
    return '0' * (len(b) - 1) + b


def payload(vps, ids):
    bits = f'{vps:04b}' + '01' + ue(len(ids) - 1) + ''.join(ue(i) for i in ids)
    if len(bits) % 8:
        bits += '1'
        bits += '0' * (-len(bits) % 8)
    return bytes(int(bits[i:i+8], 2) for i in range(0, len(bits), 8))


def main():
    seed = (DEST.parent / 'hevc' / 'main-ipb.mp4').read_bytes()
    roots = list(boxes(seed))
    root = dict(roots)
    def child(d, t):
        return next(p for k, p in boxes(d) if k == t)
    stbl = child(child(child(child(root[b'moov'], b'trak'), b'mdia'), b'minf'), b'stbl')
    sizes = child(stbl, b'stsz')
    constant, count = struct.unpack_from('>II', sizes, 4)
    assert constant == 0 and roots[-1][0] == b'moov'
    lengths = struct.unpack('>' + str(count) + 'I', sizes[12:])
    chunks = child(stbl, b'stco')
    assert int.from_bytes(chunks[4:8], 'big') == 1
    at = int.from_bytes(chunks[8:12], 'big')
    packets = []
    for n in lengths:
        packets.append(seed[at:at+n])
        at += n
    for label, p in [('config', payload(0, [0])), ('config-wrong-vps', payload(1, [0])), ('valid', payload(0, [0])), ('repeated', payload(0, [0])), ('extra-ids', payload(0, [0, 7, 15])), ('wrong-vps', payload(1, [0])), ('wrong-sps', payload(0, [1])), ('empty', b''), ('mixed', payload(0, [0]))]:
        raw = bytes([129, len(p)]) + p
        if label == 'mixed':
            raw += bytes([5, 0])
        raw += bytes([128])
        escaped = bytearray([78, 1])
        zeros = 0
        for b in raw:
            if zeros == 2 and b <= 3:
                escaped.append(3)
                zeros = 0
            escaped.append(b)
            zeros = zeros + 1 if b == 0 else 0
        output = packets.copy()
        prefix = struct.pack('>I', len(escaped)) + bytes(escaped)
        if not label.startswith('config'):
            output[0] = prefix + output[0]
        if label == 'repeated':
            output[1] = prefix + output[1]
        def configured_sei(data):
            result = bytearray(data[:23])
            at = 23
            found = False
            for _ in range(data[22]):
                header = data[at]
                count = int.from_bytes(data[at+1:at+3], 'big')
                at += 3
                units = []
                for _ in range(count):
                    size = int.from_bytes(data[at:at+2], 'big')
                    at += 2
                    units.append(data[at:at+size])
                    at += size
                if header & 63 == 39:
                    found = True
                    units.append(bytes(escaped))
                result += bytes([header]) + struct.pack('>H', len(units))
                for n in units:
                    result += struct.pack('>H', len(n)) + n
            assert at == len(data) and found
            return bytes(result)
        def rewrite(tag, data):
            if tag in (b'moov', b'trak', b'mdia', b'minf', b'stbl'):
                data = b''.join(rewrite(t, p) for t, p in boxes(data))
            elif tag == b'stsd' and label.startswith('config'):
                data = data[:8] + b''.join(box(t, p[:78] + b''.join(box(k, configured_sei(v) if k == b'hvcC' else v) for k, v in boxes(p[78:]))) for t, p in boxes(data[8:]))
            elif tag == b'stsz':
                data = data[:12] + b''.join(struct.pack('>I', len(p)) for p in output)
            elif tag == b'mdat':
                data = b''.join(output)
            return box(tag, data)
        (DEST / f'hevc-active-parameters-{label}-synthetic.mp4').write_bytes(b''.join(rewrite(t, p) for t, p in roots))


if __name__ == '__main__':
    main()
