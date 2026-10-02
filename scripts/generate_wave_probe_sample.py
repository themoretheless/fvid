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
    with wave.open(str(output / 'wave-rematrix-implicit.wav'), 'wb') as writer:
        writer.setparams((2, 2, 48000, 4800, 'NONE', 'not compressed'))
        writer.writeframes(struct.pack('<hh', 4096, 12288) * 4800)
    video = bytearray(b'YUV4MPEG2 W16 H16 F30:1 Ip C420jpeg\n')
    for value in (32, 96, 160):
        video += b'FRAME\n' + bytes([value]) * 256 + bytes([128]) * 128
    (output / 'wave-probe-info.y4m').write_bytes(video)
    (output / 'wave-rematrix-implicit.y4m').write_bytes(video)

    # Reuse only committed synthetic AAC packets; omit the deliberately
    # truncated fourth header in the existing packet-limit reproducer.
    fixtures = Path(__file__).resolve().parent.parent / 'tests/fixtures/playback-errors'
    encoded = (fixtures / 'aac-packet-prefix.aac').read_bytes()
    cursor = 0
    for _ in range(3):
        header = encoded[cursor:cursor + 7]
        assert len(header) == 7 and header[0] == 0xff and header[1] & 0xf6 == 0xf0
        size = ((header[3] & 3) << 11) | (header[4] << 3) | (header[5] >> 5)
        assert size >= 7 and cursor + size <= len(encoded)
        cursor += size
    (output / 'aac-concat-route.aac').write_bytes(encoded[:cursor])
    (output / 'aac-concat-route.y4m').write_bytes((fixtures / 'aac-packet-prefix.y4m').read_bytes())

    # Reuse only synthetic complete AAC packets; no external encoder needed.
    # 210 packets cross the normalizer's three-second dynamic-mode boundary.
    (output / 'aac-loudnorm-long.aac').write_bytes(encoded[:cursor] * 70)
    rates = [96000, 88200, 64000, 48000, 44100, 32000, 24000, 22050, 16000, 12000, 11025, 8000, 7350]
    rate = rates[(encoded[2] >> 2) & 15]
    video = f'YUV4MPEG2 W16 H16 F{rate}:1024 Ip C420jpeg\n'.encode()
    video += (b'FRAME\n' + bytes([96]) * 256 + bytes([128]) * 128) * 210
    (output / 'aac-loudnorm-long.y4m').write_bytes(video)


    # A short float32 wide-eight layout must retain its nonstandard mask.
    samples = b''.join(struct.pack('<f', channel / 16) for _ in range(16) for channel in range(8))
    fmt = struct.pack('<HHIIHHHHI', 0xfffe, 8, 48000, 48000 * 32, 32, 32, 22, 32, 0xff)
    fmt += bytes.fromhex('0300000000001000800000aa00389b71')
    payload = b'WAVEfmt ' + struct.pack('<I', 40) + fmt
    payload += b'fact' + struct.pack('<II', 4, 16)
    payload += b'data' + struct.pack('<I', len(samples)) + samples
    (output / 'wave-float-wide-mask.wav').write_bytes(b'RIFF' + struct.pack('<I', len(payload)) + payload)
    (output / 'wave-float-wide-mask.y4m').write_bytes(
        b'YUV4MPEG2 W16 H16 F3000:1 Ip C420jpeg\nFRAME\n' + bytes([96]) * 256 + bytes([128]) * 128)


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path, default=Path('tests/fixtures/playback-errors'))
    generate(parser.parse_args().output)
