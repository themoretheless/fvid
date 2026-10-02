#!/usr/bin/env python3
"""Generate a synthetic WAVE INFO regression and a matching 0.1 s video."""
import argparse
from pathlib import Path
import struct
import wave


def generate(output):
    output.mkdir(parents=True, exist_ok=True)
    audio = output / 'wave-probe-info.wav'
    with wave.open(str(audio), 'wb') as writer:
        writer.setparams((1, 2, 48000, 4800, 'NONE', 'not compressed'))
        writer.writeframes(struct.pack('<hh', 128, -128) * 2400)
    data = bytearray(audio.read_bytes())
    text = b'Synthetic probe regression\0'
    entry = b'INAM' + struct.pack('<I', len(text)) + text + b'\0' * (len(text) & 1)
    payload = b'INFO' + entry
    data += b'LIST' + struct.pack('<I', len(payload)) + payload
    data[4:8] = struct.pack('<I', len(data) - 8)
    audio.write_bytes(data)
    video = bytearray(b'YUV4MPEG2 W16 H16 F30:1 Ip C420jpeg\n')
    for value in (32, 96, 160):
        video += b'FRAME\n' + bytes([value]) * 256 + bytes([128]) * 128
    (output / 'wave-probe-info.y4m').write_bytes(video)


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path, default=Path('tests/fixtures/playback-errors'))
    generate(parser.parse_args().output)
