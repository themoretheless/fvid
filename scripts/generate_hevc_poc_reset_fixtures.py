#!/usr/bin/env python3
"""Owned offline POC-wrap/reset headers on documented synthetic HEVC seeds.

Metadata comes from our Rust parser; no external codec or private input is used.
Generation is explicit and separate from ordinary test execution.
"""
import json
import struct
import subprocess
import argparse
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
    assert chroma in (0, 1), 'monochrome or 4:2:0 source required'
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



def reset_vps(nal, aligned, chroma, malformed=None):
    value = bits(unescape(nal[2:]))
    body = value[:value.rfind('1')]
    assert body[-1] == '0', 'seed VPS extension flag must be absent'
    body = body[:-1] + '1'
    padding = -len(body) % 8
    assert padding > 0
    body += ('0' if malformed == 'bad-vps-alignment' else '1') + '1' * (padding - 1)
    # Annex F.7.3.2.1.1: internal single layer and one output layer set.
    body += ('1' if malformed == 'bad-vps-splitting' else '0') + '0' * 16 + '0' + '0000' + '000' + ue(0) + ue(0)
    body += f'{0 if malformed == "bad-vps-dimensions" else 64:016b}{64:016b}'
    if malformed == 'bad-vps-no-format': return nal[:2] + escape(packed(body + '0' + '1'))
    body += '1' + f'{chroma:02b}' + '00000000' + '0'
    body += '0' + str(int(aligned)) + ue(0) + '0' + ue(0) + '0' + '0'
    return nal[:2] + escape(packed(body + '1'))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--chroma420', action='store_true', help='generate the disjoint 4:2:0 family')
    parser.add_argument('--longterm', action='store_true', help='generate the disjoint explicit long-term family')
    parser.add_argument('--multislice', action='store_true', help='generate the disjoint four-slice reset agreement family')
    parser.add_argument('--dependent', action='store_true', help='generate one independent and three dependent segment reset family')
    parser.add_argument('--vps-extension', choices=['aligned', 'unaligned'], help='author strict Annex F single-layer VPS syntax')
    parser.add_argument('--explicit-cycle', action='store_true', help='qualify explicit cycles with aligned single-layer VPS')
    args = parser.parse_args()
    assert not args.explicit_cycle or args.vps_extension == 'aligned', 'explicit cycles require aligned VPS'
    root = Path(__file__).resolve().parents[1]
    assert sum((args.chroma420, args.longterm, args.multislice, args.dependent)) <= 1, 'choose one family'
    seed = DEST / ('hevc-chroma-qp-list-dependent-rext8.mp4' if args.dependent else 'hevc-chroma-qp-list-slices-rext8.mp4' if args.multislice else 'hevc-long-term-rext8.mp4' if args.longterm else 'hevc-pcm-reference-rext8.mp4' if args.chroma420 else 'hevc-pcm-mono-reference-rext8.mp4')
    stem = 'hevc-poc-dependent' if args.dependent else 'hevc-poc-multislice' if args.multislice else 'hevc-poc-long-term' if args.longterm else 'hevc-poc-420' if args.chroma420 else 'hevc-poc'
    if args.vps_extension: stem += '-vps-' + args.vps_extension
    if args.explicit_cycle: stem += '-explicit-cycle'
    meta = json.loads(subprocess.check_output([
        'cargo', 'run', '--quiet', '--offline', '--no-default-features',
        '--example', 'hevc_colour_plane_fixture_meta', '--', str(seed)], cwd=root))
    data = seed.read_bytes()
    roots = dict(boxes(data))
    stbl = child(child(child(child(roots[b'moov'], b'trak'), b'mdia'), b'minf'), b'stbl')
    table = child(stbl, b'stsz')
    constant, count = struct.unpack_from('>II', table, 4)
    assert constant == 0 and count == 3 and meta['dependent_enabled'] == args.dependent
    sizes = struct.unpack('>3I', table[12:])
    offset = struct.unpack_from('>I', child(stbl, b'stco'), 8)[0]
    packets = []
    for size in sizes:
        packets.append(data[offset:offset + size]); offset += size
    entry_kind, entry = next(boxes(child(stbl, b'stsd')[8:]))
    original_config = child(entry[78:], b'hvcC')
    cases = []
    labels = ['baseline', 'msb-reset', 'msb-reset-signalled', 'idr-baseline', 'full-reset',
              'bad-full-nonidr', 'bad-period-repeat', 'bad-anchor', 'bad-truncated', 'bad-empty']
    if args.multislice or args.dependent: labels += ['bad-slice-period', 'bad-slice-reset']
    if args.vps_extension: labels += ['bad-vps-alignment', 'bad-vps-splitting', 'bad-vps-dimensions', 'bad-vps-no-format']
    if args.explicit_cycle: labels = ['msb-reset', 'msb-reset-signalled', 'full-reset']
    for label in labels:
        reset = label not in ('baseline', 'idr-baseline')
        idr_family = label in ('idr-baseline', 'full-reset')
        config = bytearray(original_config[:23])
        at = 23
        original_width = None
        for _ in range(original_config[22]):
            header = original_config[at:at + 3]; at += 3; config += header
            for _ in range(int.from_bytes(header[1:], 'big')):
                n = int.from_bytes(original_config[at:at + 2], 'big'); at += 2
                nal = original_config[at:at + n]; at += n
                if header[0] & 63 == 32 and args.vps_extension:
                    nal = reset_vps(nal, args.vps_extension == 'aligned', int(args.chroma420 or args.multislice or args.dependent or args.longterm), label)
                if header[0] & 63 == 33: nal, original_width = sps_width(nal)
                if header[0] & 63 == 34 and reset: nal = reset_pps(nal)
                config += len(nal).to_bytes(2, 'big') + nal
        assert original_width is not None and at == len(original_config)
        outputs = []
        for index in range(35):
            epoch = index - (33 if index >= 33 else 17 if index >= 17 else 0) if idr_family else index
            source = min(epoch, 2)
            units = packet_units(packets[source])
            assert len(units) == len(meta['packets'][source])
            if args.multislice or args.dependent: assert len(units) == 4
            out = []
            for unit, info in zip(units, meta['packets'][source]):
                nal = unit[4:]
                assert not info['dependent'] or args.dependent
                assert info['first'] or args.multislice or args.dependent
                value = bits(bytes(info['rbsp']))
                end = info['entropy_byte_offset'] * 8
                alignment = value[:end].rfind('1')
                r = Reader(value)
                r.read(1)
                if info['idr']: r.read(1)
                r.ue()
                if not info['first']:
                    if meta['dependent_enabled']: assert bool(r.read(1)) == info['dependent']
                    r.read((meta['ctu_count'] - 1).bit_length())
                if not info['dependent']:
                    r.read(meta['extra_bits']); r.ue()
                    if meta['output_flag']: r.read(1)
                prefix = value[:r.at]
                if not info['idr'] and not info['dependent']:
                    prefix += f'{epoch % 16:04b}'
                    r.read(original_width)
                    if args.longterm:
                        start = r.at
                        assert r.read(1) == 0, 'slice-local short-term set'
                        if meta['short_term_sets']: assert r.read(1) == 0, 'nonpredicted RPS'
                        assert r.ue() == 0 and r.ue() == 0, 'no short-term entries'
                        assert meta['long_term_entries'] == 0 and r.ue() == 1
                        lsb_at = r.at
                        r.read(original_width)
                        assert r.read(1) == 1 and r.read(1) == 1
                        r.ue()
                        target = epoch - 1
                        cycle = int(target % 16 > epoch % 16)
                        prefix += value[start:lsb_at] + f'{target % 16:04b}' + '11' + ue(cycle)
                prefix += value[r.at:alignment]
                if reset:
                    extension = '00'
                    if index in (17, 33): extension = '01' + '000111'
                    if label.endswith('signalled') and index == 18:
                        extension = '11' + '000111' + '0' + '0001'
                    if label == 'full-reset':
                        period = 2 if index >= 33 else 1 if index >= 17 else 0
                        if epoch == 0: extension = '10' + f'{period:06b}'
                        elif epoch == 1: extension = '11' + f'{period:06b}' + '1' + '0000'
                        else: extension = '00'
                    if label == 'bad-full-nonidr' and index == 17: extension = '10' + '000111'
                    if label == 'bad-period-repeat' and index == 18: extension = '01' + '000111'
                    if label == 'bad-anchor' and index == 18: extension = '11' + '000111' + '1' + '0001'
                    if label == 'bad-truncated' and index == 18: extension = '11' + '000111'
                    if label == 'bad-empty' and index == 17: extension = ''
                    if index == 17 and not info['first']:
                        if label == 'bad-slice-period': extension = '01' + '001000'
                        if label == 'bad-slice-reset': extension = '00'
                    if extension and args.vps_extension == 'aligned' and not (label == 'bad-truncated' and index == 18):
                        if args.explicit_cycle:
                            # Cycle is the pre-reset MSB on an initial reset,
                            # otherwise the current POC MSB in the resetting period.
                            cycle = epoch // 16 if idr_family else (index - (32 if index > 33 else 16 if index > 17 else 0)) // 16
                            extension += '1' + ue(cycle)
                        else: extension += '0'
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
            elif tag == b'stss' and idr_family: payload = bytes(4) + struct.pack('>IIII', 3, 1, 18, 34)
            elif tag == b'stsd': payload = payload[:8] + box(entry_kind, entry[:78] + b''.join(box(t, bytes(config) if t == b'hvcC' else p) for t, p in boxes(entry[78:])))
            elif tag in (b'mvhd', b'mdhd'): payload = payload[:16] + struct.pack('>I', len(outputs)) + payload[20:]
            elif tag == b'tkhd': payload = payload[:20] + struct.pack('>I', len(outputs)) + payload[24:]
            return box(tag, payload)

        name = f'{stem}-{label}-synthetic.mp4'
        (DEST / name).write_bytes(box(b'ftyp', roots[b'ftyp']) + box(b'mdat', b''.join(outputs)) + rewrite(b'moov', roots[b'moov']))
        cases.append(dict(file=name, expectation='invalid' if label.startswith('bad-') else 'acceptance',
                          frames=len(outputs), resets=[] if not reset else ([0, 17, 33] if idr_family else [17, 33]),
                          signalled=[1, 18, 34] if label == 'full-reset' else [18] if label.endswith('signalled') else [],
                          failure_picture=(0 if label.startswith('bad-vps-') else 17 if label in ('bad-full-nonidr', 'bad-empty', 'bad-slice-period', 'bad-slice-reset') else 18) if label.startswith('bad-') else None))
    (DEST / f'{stem}-reset.json').write_text(json.dumps(dict(cases=cases, seed=seed.name,
        provenance='Original documented synthetic HEVC seed; owned POC width, slice, PPS and MP4 rewriting. No private media, FFmpeg, external codec or network. Baseline and reset acceptance compare pixels, exact POC translations and software seek/rewind; This family qualifies MSB reset, full reset at IDR and same-period signalled reset; malformed cases are Invalid refusals, not acceptance.'), indent=2) + '\n')


if __name__ == '__main__': main()
