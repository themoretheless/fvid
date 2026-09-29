#!/usr/bin/env python3
"""Generate synthetic AAC-LC/960 fixtures; no encoder or decoder is required.

Uses FVid's checked-in normative Huffman numbers. All scale-factor bands carry
nonzero book-1 coefficients, including the last band. Window transitions cover
sine/KBD, long-start, grouped eight-short and long-stop. PCM reference generation
is a separate test-only FFmpeg step documented beside the fixtures.
"""
from pathlib import Path
import re
import struct

ROOT = Path(__file__).resolve().parents[1]
tables = (ROOT / 'src/codec/aac_huffman_tables.rs').read_text()
def table(name):
    text = re.search(r'const ' + name + r':[^=]+ = \[(.*?)\];', tables, re.S)[1]
    return [int(n, 0) for n in re.findall(r'0x[0-9a-fA-F]+|\d+', text)]
codes = table('SPECTRUM_CODEBOOK1_CODES')
lens = table('SPECTRUM_CODEBOOK1_LENS')
scale_codes = table('SCF_CODEBOOK_CODES')
scale_lens = table('SCF_CODEBOOK_LENS')
band_source = (ROOT / 'src/codec/aac_band_tables.rs').read_text()
def offsets(rate, short):
    family = {48000: 48, 96000: 64 if short else 96, 8000: 8}[rate]
    name = f'SWB_OFFSET_{family}K_' + ('SHORT' if short else 'LONG')
    data = re.search(r'const ' + name + r':[^=]+ = \[(.*?)\];', band_source, re.S)[1]
    end = 120 if short else 960
    return [int(n) for n in re.findall(r'\d+', data) if int(n) < end] + [end]
def packet(sequence, shape, long_bands, short_bands, positive, rate):
    fields = [(0, 3), (0, 4), (140, 8), (0, 1), (sequence, 2), (shape, 1)]
    short = sequence == 2
    bands = short_bands if short else long_bands
    fields += [(bands, 4 if short else 6), (127, 7) if short else (0, 1)]
    fields += [(1, 4)]  # One section: signed quadruples, Huffman book 1.
    width = 3 if short else 5
    escape = (1 << width) - 1
    left = bands
    while left >= escape:
        fields += [(escape, width)]
        left -= escape
    fields += [(left, width)]
    # Different gains per band expose incorrect boundaries in the PCM oracle.
    fields += [(scale_codes[59 + band % 3], scale_lens[59 + band % 3]) for band in range(bands)]
    fields += [(0, 1)] * 3  # No pulse, TNS or gain control.
    edges = offsets(rate, short)
    assert len(edges) == bands + 1
    for band, (start, end) in enumerate(zip(edges, edges[1:])):
        for window in range(8 if short else 1):
            code = 80 if positive ^ bool(band % 2) ^ bool(window % 2) else 0
            fields += [(codes[code], lens[code])] * ((end - start) // 4)
    fields += [(7, 3)]  # ID_END and zero alignment padding.
    bits = ''.join(f'{value:0{width}b}' for value, width in fields)
    bits += '0' * (-len(bits) % 8)
    return int(bits, 2).to_bytes(len(bits) // 8, 'big')
def element(tag, data):
    width = next(w for w in range(1, 9) if len(data) < (1 << (7*w))-1)
    return tag.to_bytes((tag.bit_length()+7)//8, 'big') + ((1 << (7*width)) | len(data)).to_bytes(width, 'big') + data
def number(tag, value):
    return element(tag, value.to_bytes(max(1, (value.bit_length()+7)//8), 'big'))
for rate, index, long_bands, short_bands in [(48000,3,49,14), (96000,0,40,12), (8000,11,40,15)]:
    step = 960 * 1000 // rate
    asc = bytes([(2 << 3) | (index >> 1), ((index & 1) << 7) | (1 << 3) | 4])
    header = element(0x1a45dfa3, element(0x4282, b'matroska') + number(0x4287, 4) + number(0x4285, 2))
    info = element(0x1549a966, number(0x2ad7b1, 1_000_000) + element(0x4489, struct.pack('>d', step*7)))
    audio = element(0xe1, element(0xb5, struct.pack('>d', rate)) + number(0x9f, 1))
    track = element(0xae, number(0xd7, 1) + number(0x73c5, 1) + number(0x83, 2) + element(0x86, b'A_AAC') + element(0x63a2, asc) + number(0x23e383, step*1_000_000) + audio)
    blocks = number(0xe7, 0)
    for i, (sequence, shape) in enumerate([(0,0), (1,1), (2,0), (2,1), (3,1), (0,0), (0,0)]):
        data = packet(sequence, shape, long_bands, short_bands, i % 2 == 0, rate)
        blocks += element(0xa3, b'\x81' + struct.pack('>hB', i*step, 0x80) + data)
    result = header + element(0x18538067, info + element(0x1654ae6b, track) + element(0x1f43b675, blocks))
    path = ROOT / f'tests/fixtures/audio/aac-960-{rate}.mka'
    path.write_bytes(result)
    print(path.name, len(result))
