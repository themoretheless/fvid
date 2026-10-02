#!/usr/bin/env python3
"""Generate a synthetic half-second video/audio pair for loudnorm histogram gain.

No encoder, external process, network or private media is used. The 192 kHz
PCM alternates +/-328 (signed 16-bit), with a closed-form Nyquist weighting.
Its steady weighted level is -36.67185 LUFS: the 0.1 LU bin center is -36.65.
The paired 16x16 Y4M video alternates white/black for 15 frames at 30 fps.
"""
import argparse
from pathlib import Path
import struct
import wave

def generate(output):
    output.mkdir(parents=True, exist_ok=True)
    with wave.open(str(output / 'loudnorm-histogram.wav'), 'wb') as audio:
        audio.setparams((1, 2, 192000, 96000, 'NONE', 'not compressed'))
        audio.writeframes(struct.pack('<hh', 328, -328) * 48000)
    video = bytearray(b'YUV4MPEG2 W16 H16 F30:1 Ip A1:1 C420jpeg\n')
    for frame in range(15):
        video += b'FRAME\n' + bytes([235 if frame % 2 == 0 else 16]) * 256 + bytes([128]) * 128
    (output / 'loudnorm-histogram.y4m').write_bytes(video)

if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path, default=Path(__file__).resolve().parents[1] / 'tests/fixtures/playback-errors')
    generate(parser.parse_args().output)
