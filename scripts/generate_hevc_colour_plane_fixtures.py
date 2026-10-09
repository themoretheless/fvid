#!/usr/bin/env python3
"""Author separate-colour-plane headers from our monochrome PCM seed, offline."""
import json
import struct
import subprocess
import sys
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


def packet_units(packet):
    units = []
    pos = 0
    while pos < len(packet):
        length = int.from_bytes(packet[pos:pos+4], 'big')
        assert length >= 2 and pos+4+length <= len(packet)
        units.append(packet[pos:pos+4+length])
        pos += 4+length
    return units


def transform(plane, value):
    return value if plane == 0 else ((value+37) % 256 if plane == 1 else 255-value)


def distinct_pcm(unit, gold, plane, depth=8):
    """Replace only exactly located authored 32x32 PCM sample blocks."""
    assert len(gold) == 64*64*(1 if depth == 8 else 2)
    samples = list(gold) if depth == 8 else struct.unpack('<4096H', gold)
    blocks = [packed(''.join(f'{v:0{depth}b}' for y in range(top, top+32) for v in samples[y*64+x:y*64+x+32]))
              for top in (0, 32) for x in (0, 32)]
    payload = bytearray(unescape(unit[6:]))
    spans = []
    for block in blocks:
        pos = payload.find(block)
        assert pos >= 0 and payload.find(block, pos+1) < 0
        spans.append((pos, block))
    assert all(a+len(block) <= b for (a, block), (b, _) in zip(sorted(spans), sorted(spans)[1:]))
    for pos, block in spans:
        value = bits(block)
        values = [int(value[i:i+depth], 2) for i in range(0, len(value), depth)]
        new = [transform(plane, b) if depth == 8 else depth_transform(plane, b, depth) for b in values]
        payload[pos:pos+len(block)] = packed(''.join(f'{v:0{depth}b}' for v in new))
    nal = unit[4:6] + escape(payload)
    return len(nal).to_bytes(4, 'big') + nal


def depth_transform(plane, value, depth):
    mask = (1 << depth)-1
    return (value+(1 if plane == 0 else 37)) & mask if plane < 2 else mask-value


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
    seed = DEST / (sys.argv[1] if len(sys.argv) > 1 else 'hevc-pcm-mono-active-rext8.mp4')
    meta = json.loads(subprocess.check_output([
        'cargo', 'run', '--quiet', '--locked', '--offline', '--no-default-features',
        '--features', 'media,player', '--example', 'hevc_colour_plane_fixture_meta', '--', str(seed)]))
    root = dict(boxes(seed.read_bytes()))
    track = child(root[b'moov'], b'trak')
    stbl = child(child(child(track, b'mdia'), b'minf'), b'stbl')
    table = child(stbl, b'stsz')
    constant, count = struct.unpack_from('>II', table, 4)
    assert constant == 0
    sizes = struct.unpack('>' + str(count) + 'I', table[12:])
    offset = struct.unpack_from('>I', child(stbl, b'stco'), 8)[0]
    outputs = []
    for size, metadata in zip(sizes, meta['packets'], strict=True):
        packet = seed.read_bytes()[offset:offset+size]; offset += size
        units = packet_units(packet)
        assert len(units) == len(metadata)
        planes = []
        for plane in range(3):
            for unit, s in zip(units, metadata, strict=True):
                nal = unit[4:]
                if s['dependent']:
                    # Dependent syntax inherits the preceding independent
                    # segment's plane ID; no colour_plane_id is present here.
                    planes.append(nal)
                    continue
                value = bits(bytes(s['rbsp']))
                end = s['entropy_byte_offset']*8
                align = value[:end].rfind('1')
                insert = ue_end(value, 2 if s['idr'] else 1)
                if not s['first']:
                    insert += int(meta['dependent_enabled'])
                    insert += (meta['ctu_count']-1).bit_length()
                insert += meta['extra_bits']
                insert = ue_end(value, insert) + int(meta['output_flag'])
                header = value[:insert] + f'{plane:02b}' + value[insert:align] + '1'
                planes.append(nal[:2] + escape(packed(header) + bytes(s['rbsp'][end//8:])))
        outputs.append(b''.join(len(n).to_bytes(4, 'big') + n for n in planes))

    def rewrite(tag, payload):
        if tag in (b'moov', b'trak', b'mdia', b'minf', b'stbl'):
            payload = b''.join(rewrite(t, p) for t, p in boxes(payload))
        elif tag == b'stsz':
            payload = payload[:12] + b''.join(struct.pack('>I', len(output)) for output in outputs)
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
            # Retain the SPS-signalled chroma bit depth in hvcC. Separate
            # planes use the luma decoding path; this does not rewrite the SPS
            # bit_depth_chroma_minus8 syntax element.
            config[18] = (config[18] & 248) | (meta['chroma_depth'] - 8)
            config = bytes(config[:23] + arrays)
            payload = payload[:8] + box(kind, entry[:78] + b''.join(box(t, config if t == b'hvcC' else p) for t, p in boxes(entry[78:])))
        return box(tag, payload)

    movie = box(b'ftyp', root[b'ftyp']) + box(b'mdat', b''.join(outputs)) + rewrite(b'moov', root[b'moov'])
    name = sys.argv[2] if len(sys.argv) > 2 else 'hevc-separate-colour-planes-pcm-synthetic.mp4'
    (DEST / name).write_bytes(movie)
    mode = sys.argv[3] if len(sys.argv) > 3 else None
    assert mode in (None, 'distinct-reference', 'distinct-depth')
    if mode == 'distinct-reference':
        assert len(outputs) == 3
        gold = seed.with_suffix('.yuv').read_bytes()
        assert len(gold) == 3*4096
        units = packet_units(outputs[0])
        outputs[0] = b''.join(distinct_pcm(units[p], gold[:4096], p) for p in (2, 0, 1))
        # Vary the first plane of each AU so reference selection cannot rely
        # on either the first header or the preceding packet's ordering.
        for frame, order in [(1, (1, 2, 0)), (2, (0, 2, 1))]:
            units = packet_units(outputs[frame])
            outputs[frame] = b''.join(units[p] for p in order)
        movie = box(b'ftyp', root[b'ftyp']) + box(b'mdat', b''.join(outputs)) + rewrite(b'moov', root[b'moov'])
        (DEST / name).write_bytes(movie)
        expected = b''.join(bytes(transform(p, b) for b in gold[frame*4096:(frame+1)*4096])
                            for frame in range(3) for p in range(3))
        (DEST / name).with_suffix('.yuv').write_bytes(expected)
    if mode == 'distinct-depth':
        assert len(outputs) == 1
        depth = meta['depth']
        assert depth in (10, 12) and meta['pcm_depth'] == depth
        gold = seed.with_suffix('.yuv').read_bytes()
        units = packet_units(outputs[0])
        outputs[0] = b''.join(distinct_pcm(unit, gold, plane, depth) for plane, unit in enumerate(units))
        movie = box(b'ftyp', root[b'ftyp']) + box(b'mdat', b''.join(outputs)) + rewrite(b'moov', root[b'moov'])
        (DEST / name).write_bytes(movie)
        values = struct.unpack('<4096H', gold)
        expected = b''.join(struct.pack('<H', depth_transform(p, v, depth)) for p in range(3) for v in values)
        (DEST / name).with_suffix('.yuv').write_bytes(expected)
    if len(sys.argv) == 1:
        original = outputs[0]
        units = packet_units(original)
        for suffix, order in [('reordered', [2, 0, 1]), ('missing', [0, 1]), ('duplicate', [0, 1, 2, 0])]:
            outputs[0] = b''.join(units[i] for i in order)
            movie = box(b'ftyp', root[b'ftyp']) + box(b'mdat', b''.join(outputs)) + rewrite(b'moov', root[b'moov'])
            (DEST / f'hevc-separate-colour-planes-{suffix}-synthetic.mp4').write_bytes(movie)
        # Distinct planes expose accidental luma reuse or plane-ID swapping.
        # This authored seed has four unfiltered 32x32 PCM blocks. Locate only
        # exact saved blocks; never guess byte offsets in an entropy payload.
        gold = seed.with_suffix('.yuv').read_bytes()
        assert len(gold) == 64*64
        distinct = []
        expected = []
        for plane, unit in enumerate(units):
            distinct.append(distinct_pcm(unit, gold, plane))
            expected.append(bytes(transform(plane, b) for b in gold))
        outputs[0] = b''.join(distinct)
        movie = box(b'ftyp', root[b'ftyp']) + box(b'mdat', b''.join(outputs)) + rewrite(b'moov', root[b'moov'])
        (DEST / 'hevc-separate-colour-planes-distinct-synthetic.mp4').write_bytes(movie)
        (DEST / 'hevc-separate-colour-planes-distinct-synthetic.yuv').write_bytes(b''.join(expected))


if __name__ == '__main__':
    main()
