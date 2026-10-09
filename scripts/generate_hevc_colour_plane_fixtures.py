#!/usr/bin/env python3
"""Author separate-colour-plane headers from our monochrome PCM seed, offline."""
import json
import struct
import subprocess
from generate_he_aac_packet_fixtures import DEST, boxes
from hevc_fixture_mp4 import box


def child(data, tag):
    return next(p for t, p in boxes(data) if t == tag)


def unescape(data):
    return data.replace(b'\x00\x00\x03', b'\x00\x00')


def escape(data):
    out = bytearray()
    zeros = 0
    for b in data:
        if zeros == 2 and b <= 3:
            out.append(3)
            zeros = 0
        out.append(b)
        zeros = zeros + 1 if b == 0 else 0
    return bytes(out)


def bits(data):
    return ''.join(f'{b:08b}' for b in data)


def packed(value):
    value += '0' * (-len(value) % 8)
    return bytes(int(value[i:i+8], 2) for i in range(0, len(value), 8))


def ue_end(value, start):
    n = 0
    while value[start+n] == '0':
        n += 1
    return start + 2*n + 1


def parameter(nal):
    kind = (nal[0] >> 1) & 63
    if kind not in (32, 33):
        return nal
    value = bits(unescape(nal[2:]))
    start = 32 if kind == 32 else 8
    # Main RExt, unconstrained chroma/depth. Keep source flags and level.
    value = value[:start+3] + '00100' + '00001000000000000000000000000000' + value[start+40:start+44] + '0'*44 + value[start+88:]
    if kind == 33:
        assert value[4:7] == '000'
        chroma = ue_end(value, 104)
        assert value[chroma] == '1'  # authored seed is monochrome
        value = value[:chroma] + '001001' + value[chroma+1:]
    value = value[:value.rfind('1')+1]
    return nal[:2] + escape(packed(value))


def main():
    seed = DEST / 'hevc-pcm-mono-active-rext8.mp4'
    meta = json.loads(subprocess.check_output([
        'cargo', 'run', '--quiet', '--locked', '--offline', '--no-default-features',
        '--features', 'media,player', '--example', 'hevc_colour_plane_fixture_meta', '--', str(seed)]))
    assert len(meta['slices']) == 1
    root = dict(boxes(seed.read_bytes()))
    track = child(root[b'moov'], b'trak')
    stbl = child(child(child(track, b'mdia'), b'minf'), b'stbl')
    size = struct.unpack_from('>I', child(stbl, b'stsz'), 12)[0]
    offset = struct.unpack_from('>I', child(stbl, b'stco'), 8)[0]
    packet = seed.read_bytes()[offset:offset+size]
    n = int.from_bytes(packet[:4], 'big')
    assert n == len(packet)-4
    nal = packet[4:]
    s = meta['slices'][0]
    value = bits(bytes(s['rbsp']))
    end = s['entropy_byte_offset']*8
    align = value[:end].rfind('1')
    insert = ue_end(value, 2 if s['idr'] else 1)
    insert += meta['extra_bits']
    insert = ue_end(value, insert) + int(meta['output_flag'])
    planes = []
    for plane in range(3):
        header = value[:insert] + f'{plane:02b}' + value[insert:align] + '1'
        planes.append(nal[:2] + escape(packed(header) + bytes(s['rbsp'][end//8:])))
    output = b''.join(len(n).to_bytes(4, 'big') + n for n in planes)

    def rewrite(tag, payload):
        if tag in (b'moov', b'trak', b'mdia', b'minf', b'stbl'):
            payload = b''.join(rewrite(t, p) for t, p in boxes(payload))
        elif tag == b'stsz':
            payload = payload[:12] + struct.pack('>I', len(output))
        elif tag == b'stco':
            payload = payload[:8] + struct.pack('>I', len(root[b'ftyp'])+16)
        elif tag == b'stsd':
            kind, entry = next(boxes(payload[8:]))
            config = bytearray(child(entry[78:], b'hvcC'))
            arrays = bytearray()
            pos = 23
            for _ in range(config[22]):
                arrays += config[pos:pos+3]
                count = int.from_bytes(config[pos+1:pos+3], 'big')
                pos += 3
                for _ in range(count):
                    length = int.from_bytes(config[pos:pos+2], 'big'); pos += 2
                    unit = parameter(bytes(config[pos:pos+length])); pos += length
                    arrays += len(unit).to_bytes(2, 'big') + unit
                    if (unit[0] >> 1) & 63 == 33:
                        config[1:13] = unescape(unit[2:])[1:13]
            config[16] = (config[16] & 252) | 3
            config[18] = config[17]
            config = bytes(config[:23] + arrays)
            payload = payload[:8] + box(kind, entry[:78] + b''.join(box(t, config if t == b'hvcC' else p) for t, p in boxes(entry[78:])))
        return box(tag, payload)

    movie = box(b'ftyp', root[b'ftyp']) + box(b'mdat', output) + rewrite(b'moov', root[b'moov'])
    (DEST / 'hevc-separate-colour-planes-pcm-synthetic.mp4').write_bytes(movie)


if __name__ == '__main__':
    main()
