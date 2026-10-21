#!/usr/bin/env python3
"""Wrap owned independent AV1 key pictures with received CLL metadata; offline."""
import json
import struct
from generate_av1_configuration_samples import D, units
from generate_av1_spatial_container_samples import mp4, webm

OUT = D / 'av1-hdr-sequence'
SEED = 'av1-separate-tiles-w128-h128-groups2'
LIGHT = [(1000, 400), (1400, 500), (750, 250)]

def record_from_seed(data):
    containers = {b'moov': 0, b'trak': 0, b'mdia': 0, b'minf': 0,
                  b'stbl': 0, b'stsd': 8, b'av01': 78}
    at = 0
    while at < len(data):
        size, kind = struct.unpack('>I4s', data[at:at+8])
        assert size >= 8 and at + size <= len(data)
        payload = data[at+8:at+size]
        if kind == b'av1C':
            return payload
        if kind in containers:
            found = record_from_seed(payload[containers[kind]:])
            if found is not None:
                return found
        at += size
    assert at == len(data)
    return None

def generate():
    parts = list(units((D / (SEED + '.obu')).read_bytes()))
    seq = next(raw for kind, raw in parts if kind == 1)
    record = record_from_seed((D / (SEED + '.mp4')).read_bytes())
    assert record is not None and record[4:] == seq
    packets = []
    for cll, fall in LIGHT:
        # METADATA_TYPE_HDR_CLL (1), two big-endian u16 values, trailing bits.
        metadata = b'\x2a\x06\x01' + struct.pack('>HH', cll, fall) + b'\x80'
        packets.append(b''.join(raw + (metadata if kind == 1 else b'') for kind, raw in parts))
    files = {f'{i}.packet': packet for i, packet in enumerate(packets)}
    for ext, wrap in [('mp4', mp4), ('webm', webm)]:
        files['sequence.' + ext] = wrap(packets, seq, (128, 128), record=record)
    zero_packets = []
    for cll, fall in [(1000, 400), (0, 0), (750, 250)]:
        metadata = b'\x2a\x06\x01' + struct.pack('>HH', cll, fall) + b'\x80'
        zero_packets.append(b''.join(raw + (metadata if kind == 1 else b'') for kind, raw in parts))
    files.update({f'zero-{i}.packet': packet for i, packet in enumerate(zero_packets)})
    for ext, wrap in [('mp4', mp4), ('webm', webm)]:
        files['zero-sequence.' + ext] = wrap(zero_packets, seq, (128, 128), record=record)
    files['manifest.json'] = (json.dumps(dict(seed=SEED, light=LIGHT, frames=3), indent=2)+'\n').encode()
    return files

if __name__ == '__main__':
    OUT.mkdir(exist_ok=True)
    for name, data in generate().items():
        (OUT / name).write_bytes(data)
