#!/usr/bin/env python3
"""Tiny synthetic Matroska/QuickTime PCM precision fixtures; no external tools or inputs."""
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

def quicktime_pcm(codec, bits, payload, samples=None, channels=1, configuration=None):
    """One packet, two silent frames, then source and a repeated tail."""
    def box(kind, data):
        return struct.pack('>I4s', len(data) + 8, kind) + data
    def clock(length):
        data = bytearray(24)
        struct.pack_into('>II', data, 12, 48000, length)
        return bytes(data)
    samples = len(payload) // (bits // 8 * channels) if samples is None else samples
    mdat = box(b'mdat', payload)
    entry = bytearray(28)
    struct.pack_into('>H', entry, 6, 1)
    struct.pack_into('>HH', entry, 16, channels, bits)
    struct.pack_into('>I', entry, 24, 48000 << 16)
    entry += box(b'enda', struct.pack('>H', 1)) if configuration is None else box(b'wave', box(codec, configuration))
    stsd = box(b'stsd', struct.pack('>II', 0, 1) + box(codec, entry))
    stts = box(b'stts', struct.pack('>IIII', 0, 1, 1, samples))
    stsc = box(b'stsc', struct.pack('>IIIII', 0, 1, 1, 1, 1))
    stsz = box(b'stsz', struct.pack('>IIII', 0, 0, 1, len(payload)))
    stco = box(b'stco', struct.pack('>III', 0, 1, 8))
    stbl = box(b'stbl', stsd + stts + stsc + stsz + stco)
    hdlr = box(b'hdlr', bytes(8) + b'soun' + bytes(12))
    dinf = box(b'dinf', box(b'dref', struct.pack('>II', 0, 1) + box(b'url ', struct.pack('>I', 1))))
    mdia = box(b'mdia', box(b'mdhd', clock(samples)) + hdlr + box(b'minf', box(b'smhd', bytes(8)) + dinf + stbl))
    tkhd = bytearray(84)
    tkhd[3] = 7
    struct.pack_into('>H', tkhd, 36, 256)
    struct.pack_into('>I', tkhd, 12, 1)
    struct.pack_into('>I', tkhd, 20, samples * 2)
    for offset, value in [(40, 1 << 16), (56, 1 << 16), (72, 1 << 30)]:
        struct.pack_into('>I', tkhd, offset, value)
    edits = [(2, -1), (samples, 0), (samples - 2, 2)]
    elst = box(b'elst', struct.pack('>II', 0, len(edits)) + b''.join(
        struct.pack('>IiHH', length, start, 1, 0) for length, start in edits))
    trak = box(b'trak', box(b'tkhd', tkhd) + box(b'edts', elst) + mdia)
    mvhd = bytearray(100)
    mvhd[:20] = clock(samples * 2)[:20]
    struct.pack_into('>IH', mvhd, 20, 1 << 16, 256)
    for offset, value in [(36, 1 << 16), (52, 1 << 16), (68, 1 << 30), (96, 2)]:
        struct.pack_into('>I', mvhd, offset, value)
    return mdat + box(b'moov', box(b'mvhd', mvhd) + trak)

if __name__ == '__main__':
    ROOT.mkdir(parents=True, exist_ok=True)
    integers = [2147483647, -2147483647, 16777217, -16777217, 1, -1]
    for order, tag in [('little', 'LIT'), ('big', 'BIG')]:
        payload = b''.join(v.to_bytes(4, order, signed=True) for v in integers)
        (ROOT / f'pcm32-precision-{order}.mka').write_bytes(container(f'A_PCM/INT/{tag}', 32, payload))
    doubles = [0.12345678901234567, -0.9876543210987654, 1.0000000000000002, -1.0000000000000002, 1e-100, -1e-100]
    (ROOT / 'pcm64-precision.mka').write_bytes(container('A_PCM/FLOAT/IEEE', 64, b''.join(struct.pack('<d', v) for v in doubles)))
    exact = [0.12345678901234567, -0.9876543210987654,
             0.5000000000000001, -0.5000000000000001, 1e-100, -1e-100]
    (ROOT / 'pcm64-precision-edits.mov').write_bytes(quicktime_pcm(
        b'fl64', 64, b''.join(struct.pack('<d', v) for v in exact)))
    (ROOT / 'pcm32-precision-edits.mov').write_bytes(quicktime_pcm(
        b'in32', 32, b''.join(struct.pack('<i', v) for v in integers)))
