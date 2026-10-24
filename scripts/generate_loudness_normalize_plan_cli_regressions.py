#!/usr/bin/env python3
"""Generate synthetic WAVE and Matroska PCM audio fixtures for loudness/normalize-loudness plan CLI and API parity regressions (FVID-0238)."""

from __future__ import annotations

import hashlib
import json
import math
from pathlib import Path
import struct

from generate_audio_resample_window_fixture import ebml


ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / "tests/fixtures/playback-errors/loudness-normalize-plan-cli"


def u(tag: str, n: int) -> bytes:
    return ebml(tag, n.to_bytes(max(1, (n.bit_length() + 7) // 8), "big"))


def f64_elem(tag: str, val: float) -> bytes:
    return ebml(tag, struct.pack(">d", val))


def simple_block(track_no: int, rel_ms: int, payload: bytes) -> bytes:
    assert 1 <= track_no <= 126
    return ebml("a3", bytes([0x80 | track_no]) + struct.pack(">hB", rel_ms, 0x80) + payload)


def simple_tag(name: str, value: str) -> bytes:
    return ebml("67c8", ebml("45a3", name.encode("utf-8")) + ebml("4487", value.encode("utf-8")))


def info_subchunk(fourcc: bytes, text: str) -> bytes:
    raw = text.encode("utf-8") + b"\x00"
    pad = b"\x00" if len(raw) % 2 == 1 else b""
    return fourcc + struct.pack("<I", len(raw)) + raw + pad


def build_stereo_wave() -> bytes:
    sample_rate = 48_000
    channels = 2
    frames = 24_000  # 0.5 seconds
    pcm = bytearray()
    for i in range(frames):
        t = i / sample_rate
        left = int(round(math.sin(2.0 * math.pi * 440.0 * t) * 8000.0))
        right = int(round(math.cos(2.0 * math.pi * 660.0 * t) * 6000.0))
        pcm += struct.pack("<hh", left, right)

    info_body = (
        b"INFO"
        + info_subchunk(b"INAM", "Stereo Plan Wave")
        + info_subchunk(b"IART", "FVid Synth")
    )
    list_chunk = b"LIST" + struct.pack("<I", len(info_body)) + info_body
    fmt_chunk = b"fmt " + struct.pack(
        "<IHHIIHH",
        16,
        1,
        channels,
        sample_rate,
        sample_rate * channels * 2,
        channels * 2,
        16,
    )
    data_chunk = b"data" + struct.pack("<I", len(pcm)) + bytes(pcm)
    riff_body = b"WAVE" + fmt_chunk + list_chunk + data_chunk
    return b"RIFF" + struct.pack("<I", len(riff_body)) + riff_body


def build_matroska_pcm() -> bytes:
    sample_rate = 48_000
    channels = 2
    frames_per_block = 4_800  # 100 ms
    num_blocks = 5  # 500 ms total

    header = ebml("1a45dfa3", ebml("4282", b"matroska") + u("4287", 4) + u("4285", 2))
    info = ebml(
        "1549a966",
        u("2ad7b1", 1_000_000) + ebml("5741", b"FVidLoudnessPlanSynth"),
    )
    audio_settings = ebml(
        "e1",
        f64_elem("b5", float(sample_rate)) + u("9f", channels) + u("6264", 16),
    )
    track = ebml(
        "ae",
        u("d7", 1)
        + u("73c5", 801)
        + u("83", 2)
        + ebml("86", b"A_PCM/INT/LIT")
        + ebml("536e", b"MainAudio")
        + ebml("22b59c", b"eng")
        + audio_settings,
    )
    tracks = ebml("1654ae6b", track)
    tags = ebml(
        "1254c367",
        ebml(
            "7373",
            ebml("63c0", b"") + simple_tag("TITLE", "Mka Plan Source"),
        ),
    )

    blocks = bytearray()
    for b_idx in range(num_blocks):
        payload = bytearray()
        for f_idx in range(frames_per_block):
            sample_idx = b_idx * frames_per_block + f_idx
            t = sample_idx / sample_rate
            left = int(round(math.sin(2.0 * math.pi * 440.0 * t) * 8000.0))
            right = int(round(math.cos(2.0 * math.pi * 660.0 * t) * 6000.0))
            payload += struct.pack("<hh", left, right)
        blocks += simple_block(1, b_idx * 100, bytes(payload))

    cluster = ebml("1f43b675", u("e7", 0) + bytes(blocks))
    return header + ebml("18538067", info + tracks + tags + cluster)


def generate() -> dict[str, bytes]:
    OUT.mkdir(parents=True, exist_ok=True)
    files = {
        "stereo.wav": build_stereo_wave(),
        "stereo-pcm.mka": build_matroska_pcm(),
    }
    manifest = {
        "batch_id": "FVID-0238",
        "description": "Synthetic WAVE and Matroska PCM audio fixtures for loudness/normalize-loudness plan CLI and API parity regressions.",
        "files": {},
    }
    for name, content in files.items():
        path = OUT / name
        path.write_bytes(content)
        manifest["files"][name] = {
            "bytes": len(content),
            "sha256": hashlib.sha256(content).hexdigest(),
        }
    manifest_bytes = (json.dumps(manifest, indent=2, sort_keys=True) + "\n").encode("utf-8")
    (OUT / "manifest.json").write_bytes(manifest_bytes)
    files["manifest.json"] = manifest_bytes
    return files


if __name__ == "__main__":
    generate()
