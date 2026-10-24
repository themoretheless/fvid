#!/usr/bin/env python3
"""Generate synthetic WAVE audio fixtures for Batch 216 (FVID-0223):
metadata and stream controls across mix-audio, merge-audio, and Matroska concat.
"""

from __future__ import annotations

import math
import pathlib
import struct

ROOT = pathlib.Path(__file__).resolve().parents[1]
OUT_DIR = ROOT / "tests" / "fixtures" / "playback-errors" / "audio-mix-merge-concat-metadata"


def build_wave_s16le_with_info(
    sample_rate: int,
    channels: int,
    frames: int,
    freqs: list[float],
    info_entries: list[tuple[bytes, bytes]],
) -> bytes:
    pcm = bytearray()
    for frame in range(frames):
        t = frame / sample_rate
        for ch in range(channels):
            freq = freqs[ch % len(freqs)]
            val = int(round(12000.0 * math.sin(2.0 * math.pi * freq * t)))
            pcm.extend(struct.pack("<h", max(-32768, min(32767, val))))

    block_align = channels * 2
    byte_rate = sample_rate * block_align
    fmt_chunk = struct.pack(
        "<4sIHHIIHH",
        b"fmt ",
        16,
        1,
        channels,
        sample_rate,
        byte_rate,
        block_align,
        16,
    )
    data_chunk = struct.pack("<4sI", b"data", len(pcm)) + bytes(pcm)
    if len(pcm) % 2 == 1:
        data_chunk += b"\x00"

    info_body = bytearray(b"INFO")
    for tag, value in info_entries:
        assert len(tag) == 4
        raw = value + b"\x00"
        info_body.extend(tag)
        info_body.extend(struct.pack("<I", len(raw)))
        info_body.extend(raw)
        if len(raw) % 2 == 1:
            info_body.append(0)
    list_chunk = struct.pack("<4sI", b"LIST", len(info_body)) + bytes(info_body)
    if len(info_body) % 2 == 1:
        list_chunk += b"\x00"

    riff_body = b"WAVE" + fmt_chunk + data_chunk + list_chunk
    return struct.pack("<4sI", b"RIFF", len(riff_body)) + riff_body


def generate_all(out_dir: pathlib.Path = OUT_DIR) -> dict[str, bytes]:
    out_dir.mkdir(parents=True, exist_ok=True)
    files = {
        "tone-a-48k-stereo.wav": build_wave_s16le_with_info(
            sample_rate=48000,
            channels=2,
            frames=960,
            freqs=[440.0, 660.0],
            info_entries=[(b"INAM", b"Tone A"), (b"IART", b"FVid Synth")],
        ),
        "tone-b-48k-stereo.wav": build_wave_s16le_with_info(
            sample_rate=48000,
            channels=2,
            frames=960,
            freqs=[550.0, 880.0],
            info_entries=[(b"INAM", b"Tone B"), (b"IART", b"FVid Synth")],
        ),
    }
    for name, payload in files.items():
        (out_dir / name).write_bytes(payload)
    return files


if __name__ == "__main__":
    generate_all()
