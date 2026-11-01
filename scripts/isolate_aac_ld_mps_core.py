#!/usr/bin/env python3
"""Explicit research transform of the pinned original LD5151 AAC core.

Not fixture generation or a production demuxer. Requires records exported by
aac_spatial_reference and syntax JSONL from aac_ld_core_inspect. No network.
"""
import argparse
import hashlib
import json
from pathlib import Path
import struct

SOURCE_SHA = '615b5b6da42d5455982670e24e09046d978f596d1161078ea3f8e1b21137e42c'


def pack(bits):
    return int(bits + '0' * (-len(bits) % 8), 2).to_bytes((len(bits)+7)//8, 'big')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('records', type=Path)
    parser.add_argument('syntax', type=Path)
    parser.add_argument('output', type=Path)
    parser.add_argument('--report', required=True, type=Path)
    args = parser.parse_args()
    inputs = {args.records.resolve(), args.syntax.resolve()}
    outputs = {args.output.resolve(), args.report.resolve()}
    if inputs & outputs or len(outputs) != 2:
        raise ValueError('outputs must be distinct from inputs and each other')
    fixtures = Path(__file__).resolve().parents[1]/'tests/fixtures'
    if any(p.is_relative_to(fixtures) for p in outputs):
        raise ValueError('original-media transforms must stay outside fixtures')
    raw = args.records.read_bytes()
    if hashlib.sha256(raw).hexdigest() != SOURCE_SHA:
        raise ValueError('not the pinned original LD5151 records')
    at = 8

    def word():
        nonlocal at
        value = struct.unpack_from('<I', raw, at)[0]
        at += 4
        return value

    def payload():
        nonlocal at
        n = word()
        value = raw[at:at+n]
        if len(value) != n:
            raise ValueError('truncated records')
        at += n
        return value

    asc = payload()
    bits = ''.join(f'{b:08b}' for b in asc)
    # Fixed source-specific syntax position, not a generic sync-word search.
    pos = 22
    if bits[pos:pos+12] != f'{0x7cc:011b}1':
        raise ValueError('original spatial extension differs')
    core_asc = pack(bits[:pos+11]+'0')
    count = word()
    rows = [json.loads(line) for line in args.syntax.read_text().splitlines()]
    if len(rows) != count or count != 2820:
        raise ValueError('syntax AU count differs')
    out = bytearray(b'FVIDAAC1'+struct.pack('<I', len(core_asc))+core_asc
                    +struct.pack('<I', count))
    for index, row in enumerate(rows):
        packet = payload()
        k = row['core_bits']
        if (row['frame'] != index or row['frame_samples'] != 512
                or not row['spatial'] or row['has_ltp']
                or 13 in row['books'] or not 0 < k <= 8*len(packet)):
            raise ValueError('unexpected original core syntax')
        prefix = pack(''.join(f'{b:08b}' for b in packet)[:k])
        out += struct.pack('<I', len(prefix))+prefix
    if at != len(raw):
        raise ValueError('trailing records')
    if hashlib.sha256(out).hexdigest() != '11df174bf1aa8387c583c487c51c4159f5a58b059b1d0dbf3a9009d532cd3053':
        raise ValueError('isolated core differs from the qualified bit-prefix transform')
    report = dict(source_sha256=SOURCE_SHA,
                  syntax_sha256=hashlib.sha256(args.syntax.read_bytes()).hexdigest(),
                  core_records_sha256=hashlib.sha256(out).hexdigest(),
                  aus=count, frame_samples=512, asc_sync_bit=pos,
                  conformance_verdict=None,
                  operation='Research transform: disable ASC spatial extension, '
                  'retain exact core prefix bits, remove ancillary suffix, '
                  'zero-pad byte alignment. Not an unchanged original or fixture.')
    args.output.write_bytes(out)
    args.report.write_text(json.dumps(report, indent=2)+'\n')


if __name__ == '__main__':
    main()
