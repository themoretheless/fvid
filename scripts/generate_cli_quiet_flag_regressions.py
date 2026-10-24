#!/usr/bin/env python3
"""Generate tiny synthetic WAVE and Y4M fixtures for CLI quiet/memory/progress regressions."""

from __future__ import annotations

import math
import pathlib
import struct


ROOT = pathlib.Path(__file__).resolve().parents[1]
OUT_DIR = ROOT / "tests" / "fixtures" / "playback-errors" / "cli-quiet-flag"


def build_tone_wav() -> bytes:
    sample_rate = 48_000
    channels = 1
    bits_per_sample = 16
    frames = 4_800  # 100 ms
    samples = bytearray()
    for i in range(frames):
        val = int(round(12_000.0 * math.sin(2.0 * math.pi * 440.0 * i / sample_rate)))
        samples.extend(struct.pack("<h", val))
    byte_rate = sample_rate * channels * (bits_per_sample // 8)
    block_align = channels * (bits_per_sample // 8)
    fmt_chunk = struct.pack(
        "<4sIHHIIHH",
        b"fmt ",
        16,
        1,
        channels,
        sample_rate,
        byte_rate,
        block_align,
        bits_per_sample,
    )
    data_chunk = struct.pack("<4sI", b"data", len(samples)) + bytes(samples)
    riff_body = b"WAVE" + fmt_chunk + data_chunk
    return struct.pack("<4sI", b"RIFF", len(riff_body)) + riff_body


def build_tiny_y4m() -> bytes:
    header = b"YUV4MPEG2 W2 H2 F1:1 Ip A1:1 C420jpeg\n"
    frame0 = b"FRAME\n" + bytes([64, 96, 128, 160, 128, 128])
    frame1 = b"FRAME\n" + bytes([80, 112, 144, 176, 128, 128])
    return header + frame0 + frame1


def build_tiny_srt() -> bytes:
    return (
        b"1\n"
        b"00:00:00,000 --> 00:00:01,000\n"
        b"Hello world\n"
    )


def main() -> None:
    OUT_DIR.mkdir(parents=True, exist_ok=True)
    (OUT_DIR / "tone.wav").write_bytes(build_tone_wav())
    (OUT_DIR / "tiny.y4m").write_bytes(build_tiny_y4m())
    (OUT_DIR / "tiny.srt").write_bytes(build_tiny_srt())


if __name__ == "__main__":
    main()
