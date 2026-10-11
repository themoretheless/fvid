#!/usr/bin/env python3
"""Owned offline POC-wrap/reset headers on the documented synthetic PCM seed.

Metadata comes from our Rust parser; no external codec or private input is used.
Generation is explicit and separate from ordinary test execution.
"""
import json
import struct
import subprocess
from pathlib import Path
from generate_he_aac_packet_fixtures import DEST, boxes
from generate_hevc_colour_plane_fixtures import bits, packed, escape, unescape, child, packet_units
from hevc_fixture_mp4 import box


def ue(n):
    value = f'{n + 1:b}'
    return '0' * (len(value) - 1) + value


class Reader:
    def __init__(self, value, at=0):
        self.value, self.at = value, at

    def read(self, width):
        out = self.value[self.at:self.at + width]
        assert len(out) == width
        self.at += width
        return int(out, 2) if out else 0

    def ue(self):
        zeros = 0
        while self.read(1) == 0:
            zeros += 1
        return (1 << zeros) - 1 + self.read(zeros)


def sps_width(nal):
    value = bits(unescape(nal[2:]))
    assert value[4:7] == '000', 'single temporal layer required'
    r = Reader(value, 104)
    r.ue()
    chroma = r.ue()
    assert chroma == 0, 'monochrome source required'
    r.ue(); r.ue()
    if r.read(1):
        for _ in range(4): r.ue()
    r.ue(); r.ue()
    start = r.at
    original_width = r.ue() + 4
    value = value[:start] + ue(0) + value[r.at:]
    return nal[:2] + escape(packed(value[:value.rfind('1') + 1])), original_width


def reset_pps(nal):
    value = bits(unescape(nal[2:]))
    r = Reader(value)
    r.ue(); r.ue(); r.read(7); r.ue(); r.ue(); r.ue(); r.read(2)
    if r.read(1): r.ue()
    r.ue(); r.ue(); r.read(4)
    assert r.read(1) == 0, 'no tiles in seed'
    assert r.read(1) == 0, 'no entropy sync in seed'
    r.read(1)
    if r.read(1):
        r.read(1)
        if r.read(1) == 0: r.ue(); r.ue()
    assert r.read(1) == 0, 'no PPS scaling lists in seed'
    r.read(1); r.ue()
    header_extension = r.at
    assert r.read(1) == 0
    extension_present = r.at
    present = r.read(1)
    body = value[:value.rfind('1')]
    body = body[:header_extension] + '1' + body[header_extension + 1:]
    if present:
        assert body[extension_present + 1:extension_present + 9] == '10000000'
        body = body[:extension_present + 2] + '1' + body[extension_present + 3:]
    else:
        body = body[:extension_present] + '101000000'
    return nal[:2] + escape(packed(body + '1010' + '1'))


def main():
    root = Path(__file__).resolve().parents[1]
    seed = DEST / 'hevc-pcm-mono-reference-rext8.mp4'
    meta = json.loads(subprocess.check_output([
        'cargo', 'run', '--quiet', '--offline', '--no-default-features',
        '--example', 'hevc_colour_plane_fixture_meta', '--', str(seed)], cwd=root))
    data = seed.read_bytes()
    roots = dict(boxes(data))
    stbl = child(child(child(child(roots[b'moov'], b'trak'), b'mdia'), b'minf'), b'stbl')
    table = child(stbl, b'stsz')
    constant, count = struct.unpack_from('>II', table, 4)
    assert constant == 0 and count == 3 and not meta['dependent_enabled']
    sizes = struct.unpack('>3I', table[12:])
    offset = struct.unpack_from('>I', child(stbl, b'stco'), 8)[0]
    packets = []
    for size in sizes:
        packets.append(data[offset:offset + size]); offset += size
    entry_kind, entry = next(boxes(child(stbl, b'stsd')[8:]))
    original_config = child(entry[78:], b'hvcC')
    cases = []
    for label in ['baseline', 'msb-reset', 'msb-reset-signalled']:
        reset = label != 'baseline'
        config = bytearray(original_config[:23])
        at = 23
        original_width = None
        for _ in range(original_config[22]):
            header = original_config[at:at + 3]; at += 3; config += header
            for _ in range(int.from_bytes(header[1:], 'big')):
                n = int.from_bytes(original_config[at:at + 2], 'big'); at += 2
                nal = original_config[at:at + n]; at += n
                if header[0] & 63 == 33: nal, original_width = sps_width(nal)
                if header[0] & 63 == 34 and reset: nal = reset_pps(nal)
                config += len(nal).to_bytes(2, 'big') + nal
        assert original_width is not None and at == len(original_config)
        outputs = []
        for index in range(35):
            source = min(index, 2)
            units = packet_units(packets[source])
            assert len(units) == len(meta['packets'][source])
            out = []
            for unit, info in zip(units, meta['packets'][source]):
                nal = unit[4:]
                assert info['first'] and not info['dependent']
                value = bits(bytes(info['rbsp']))
                end = info['entropy_byte_offset'] * 8
                alignment = value[:end].rfind('1')
                r = Reader(value)
                r.read(1)
                if info['idr']: r.read(1)
                r.ue(); r.read(meta['extra_bits']); r.ue()
                if meta['output_flag']: r.read(1)
                prefix = value[:r.at]
                if not info['idr']:
                    prefix += f'{index % 16:04b}'
                    r.read(original_width)
                prefix += value[r.at:alignment]
                if reset:
                    extension = '00'
                    if index in (17, 33): extension = '01' + '000111'
                    if label.endswith('signalled') and index == 18:
                        extension = '11' + '000111' + '0' + '0001'
                    extension = bits(packed(extension))
                    prefix += ue(len(extension) // 8) + extension
                rewritten = nal[:2] + escape(packed(prefix + '1') + bytes(info['rbsp'][end // 8:]))
                out.append(len(rewritten).to_bytes(4, 'big') + rewritten)
            outputs.append(b''.join(out))

        def rewrite(tag, payload):
            if tag in (b'moov', b'trak', b'mdia', b'minf', b'stbl'):
                payload = b''.join(rewrite(t, p) for t, p in boxes(payload))
            elif tag == b'stsz': payload = bytes(8) + struct.pack('>I', len(outputs)) + b''.join(struct.pack('>I', len(p)) for p in outputs)
            elif tag == b'stts': payload = bytes(4) + struct.pack('>III', 1, len(outputs), 1)
            elif tag == b'stsc': payload = bytes(4) + struct.pack('>IIII', 1, 1, len(outputs), 1)
            elif tag == b'stco': payload = payload[:8] + struct.pack('>I', len(roots[b'ftyp']) + 16)
            elif tag == b'stsd': payload = payload[:8] + box(entry_kind, entry[:78] + b''.join(box(t, bytes(config) if t == b'hvcC' else p) for t, p in boxes(entry[78:])))
            elif tag in (b'mvhd', b'mdhd'): payload = payload[:16] + struct.pack('>I', len(outputs)) + payload[20:]
            elif tag == b'tkhd': payload = payload[:20] + struct.pack('>I', len(outputs)) + payload[24:]
            return box(tag, payload)

        name = f'hevc-poc-{label}-synthetic.mp4'
        (DEST / name).write_bytes(box(b'ftyp', roots[b'ftyp']) + box(b'mdat', b''.join(outputs)) + rewrite(b'moov', roots[b'moov']))
        cases.append(dict(file=name, frames=len(outputs), resets=[] if not reset else [17, 33], signalled=[18] if label.endswith('signalled') else []))
    (DEST / 'hevc-poc-reset.json').write_text(json.dumps(dict(cases=cases, seed=seed.name,
        provenance='Original documented synthetic PCM/inter seed; owned POC width, slice, PPS and MP4 rewriting. No private media, FFmpeg, external codec or network. Candidate fixtures require baseline decoding and exact reset comparison before acceptance.'), indent=2) + '\n')


if __name__ == '__main__': main()
