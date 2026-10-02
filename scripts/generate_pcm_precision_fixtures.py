#!/usr/bin/env python3
"""Tiny synthetic Matroska PCM precision fixtures; no external tools or inputs."""
from pathlib import Path
import struct

ROOT = Path(__file__).resolve().parents[1] / 'tests/fixtures/playback-errors'

def element(identifier, payload):
    return bytes.fromhex(identifier) + struct.pack('>I', len(payload) | 0x10000000) + payload

def container(codec, bits, payload):
    audio = element('e1', element('b5', struct.pack('>d', 48000)) + element('9f', b'\x02') + element('6264', bytes([bits])))
    track = element('ae', element('d7', b'\x01') + element('83', b'\x02') + element('86', codec.encode()) + audio)
    cluster = element('1f43b675', element('e7', b'\0') + element('a3', b'\x81\0\0\x80' + payload))
    return element('1a45dfa3', element('4282', b'matroska')) + element('18538067', element('1654ae6b', track) + cluster)

if __name__ == '__main__':
    ROOT.mkdir(parents=True, exist_ok=True)
    integers = [2147483647, -2147483647, 16777217, -16777217, 1, -1]
    for order, tag in [('little', 'LIT'), ('big', 'BIG')]:
        payload = b''.join(v.to_bytes(4, order, signed=True) for v in integers)
        (ROOT / f'pcm32-precision-{order}.mka').write_bytes(container(f'A_PCM/INT/{tag}', 32, payload))
    doubles = [0.12345678901234567, -0.9876543210987654, 1.0000000000000002, -1.0000000000000002, 1e-100, -1e-100]
    (ROOT / 'pcm64-precision.mka').write_bytes(container('A_PCM/FLOAT/IEEE', 64, b''.join(struct.pack('<d', v) for v in doubles)))
