#!/usr/bin/env python3
"""Generate synthetic fixtures for Batch 211 (FVID-0218) mix-audio, merge-audio, and convert-subtitles controls."""

from pathlib import Path
import struct

ROOT = Path(__file__).resolve().parent.parent
FIXTURES = ROOT / "tests" / "fixtures" / "playback-errors"


def build_wave(frames: int, rate: int = 48000, channels: int = 1, freq: float = 440.0) -> bytes:
    import math
    samples = []
    for i in range(frames):
        val = 0.25 * math.sin(2.0 * math.pi * freq * (i / rate))
        for _ in range(channels):
            samples.append(val)
    payload = b"".join(struct.pack("<f", s) for s in samples)
    byte_rate = rate * channels * 4
    block_align = channels * 4
    header = (
        b"RIFF"
        + struct.pack("<I", 36 + len(payload))
        + b"WAVEfmt "
        + struct.pack("<IHHIIHH", 16, 3, channels, rate, byte_rate, block_align, 32)
        + b"data"
        + struct.pack("<I", len(payload))
    )
    return header + payload


def build_y4m(width: int = 8, height: int = 8, frames: int = 2) -> bytes:
    header = f"YUV4MPEG2 W{width} H{height} F25:1 Ip A1:1 C420jpeg\n".encode("ascii")
    luma_size = width * height
    chroma_size = (width // 2) * (height // 2)
    out = bytearray(header)
    for i in range(frames):
        out.extend(b"FRAME\n")
        out.extend(bytes([40 + i * 20]) * luma_size)
        out.extend(bytes([128]) * chroma_size)
        out.extend(bytes([128]) * chroma_size)
    return bytes(out)


def build_srt() -> bytes:
    return (
        "1\n"
        "00:00:00,000 --> 00:00:00,500\n"
        "First synthetic subtitle cue\n\n"
        "2\n"
        "00:00:00,500 --> 00:00:01,000\n"
        "Second synthetic subtitle cue\n"
    ).encode("utf-8")


def generate(dest: Path = FIXTURES) -> dict[str, Path]:
    dest.mkdir(parents=True, exist_ok=True)
    wav_a = dest / "mix-merge-controls-a.wav"
    wav_b = dest / "mix-merge-controls-b.wav"
    video = dest / "mix-merge-controls-video.y4m"
    srt = dest / "mix-merge-controls.srt"
    wav_a.write_bytes(build_wave(4800, 48000, 1, 440.0))
    wav_b.write_bytes(build_wave(4800, 48000, 1, 660.0))
    video.write_bytes(build_y4m(8, 8, 2))
    srt.write_bytes(build_srt())
    return {
        "wav_a": wav_a,
        "wav_b": wav_b,
        "video": video,
        "srt": srt,
    }


if __name__ == "__main__":
    generate()
