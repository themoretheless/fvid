#!/usr/bin/env python3
"""Synthetic QuickTime IMA4, IMA WAV and MS ADPCM blocks; no external codecs or source media."""
from generate_pcm_precision_fixtures import ROOT, quicktime_pcm
import struct

if __name__ == '__main__':
    ROOT.mkdir(parents=True, exist_ok=True)
    # Predictor/index zero, low nibble then high nibble both 1: +1 per sample.
    packet = bytes(2) + bytes([0x11]) * 32
    (ROOT / 'ima4-ramp-edits.mov').write_bytes(quicktime_pcm(b'ima4', 16, packet, samples=64))
    fmt = struct.pack('<HHIIHHHH', 17, 2, 48000, 48000 * 16 // 9, 16, 4, 2, 9)
    stereo = bytes(4) + struct.pack('<hBB', -128, 0, 0) + bytes([0x11]) * 4 + bytes(4)
    (ROOT / 'ima-wav-stereo-edits.mov').write_bytes(quicktime_pcm(
        bytes([109, 115, 0, 17]), 16, stereo, samples=9, channels=2, configuration=fmt))
    adaptive = struct.pack('>H', 1024 | 20) + bytes((i * 73 + 19) & 255 for i in range(32))
    adaptive += struct.pack('>H', (65536 - 1024) | 60) + bytes((i * 31 + 237) & 255 for i in range(32))
    (ROOT / 'ima4-adaptive-stereo-edits.mov').write_bytes(quicktime_pcm(
        b'ima4', 16, adaptive, samples=64, channels=2))
    coefficients = [(256,0),(512,-256),(0,0),(192,64),(240,0),(460,-208),(392,-232)]
    for channels, payload, name in [
        (2, bytes([0,0]) + struct.pack('<hhhhhh',16,16,0,-128,-16,-128) + bytes([0x10])*4, 'ms-adpcm-stereo-edits.mov'),
        (1, bytes([3]) + struct.pack('<hhh',16,-3,0) + bytes([0x0f,0x78,0x41,0xe2,0xf0,0x36,0x9a,0x05]), 'ms-adpcm-adaptive-mono-edits.mov'),
    ]:
        frames = 2 + (len(payload)-7*channels)*2//channels
        fmt = struct.pack('<HHIIHHHHH',2,channels,48000,48000*len(payload)//frames,len(payload),4,32,frames,7)
        fmt += b''.join(struct.pack('<hh',a,b) for a,b in coefficients)
        (ROOT / name).write_bytes(quicktime_pcm(bytes([109,115,0,2]),16,payload,
            samples=frames,channels=channels,configuration=fmt))
