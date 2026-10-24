#!/usr/bin/env python3
"""Generate deterministic synthetic fixtures for Batch 232 (`FVID-0239`).

Covers:
1. Single-track VP9 WebM/Matroska video (`vp9-burn-source.webm`) with custom
   `WritingApp` (`"FVidVp9BurnSynth"`) for `burn-subtitles` and
   `plan burn-subtitles` admission, execution, and `WritingApp` (`ENCODER`)
   preservation.
2. Single-track rotated (`roll = -90.0` -> 90 deg clockwise) and cropped
   Matroska video (`vp9-rotated-cropped-burn.mkv`) to verify that
   `owned_subtitle_burn::prepare` uses visible post-rotation geometry rather
   than coded dimensions.
3. Plain SRT subtitle cues (`burn-cues.srt`).
4. Matching 4x4 Y4M cross-fade primary/secondary sources (`xfade-main-4x4.y4m`,
   `xfade-other-4x4.y4m`) for container metadata key/value boundary enforcement
   in `owned_xfade` and `owned_lossless::metadata`.
"""

from __future__ import annotations

import hashlib
import json
from pathlib import Path
import struct

from generate_audio_resample_window_fixture import ebml
from generate_vp9_chroma import frame as vp9_frame


ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / "tests/fixtures/playback-errors/subtitle-burn-matroska-xfade-metadata"


def u(tag: str, n: int) -> bytes:
    return ebml(tag, n.to_bytes(max(1, (n.bit_length() + 7) // 8), "big"))


def f64_elem(tag: str, val: float) -> bytes:
    return ebml(tag, struct.pack(">d", val))


def simple_tag(name: str, value: str) -> bytes:
    return ebml(
        "67c8",
        ebml("45a3", name.encode("utf-8")) + ebml("4487", value.encode("utf-8")),
    )


def build_vp9_burn_source() -> bytes:
    pkt = vp9_frame(8, "420", width=96, height=64)
    header = ebml(
        "1a45dfa3",
        ebml("4282", b"webm") + u("4287", 4) + u("4285", 2),
    )
    info = ebml(
        "1549a966",
        u("2ad7b1", 1_000_000)
        + ebml("4d80", b"FVidVp9BurnMux")
        + ebml("5741", b"FVidVp9BurnSynth"),
    )
    video = u("b0", 96) + u("ba", 64)
    track = ebml(
        "ae",
        u("d7", 1)
        + u("73c5", 101)
        + u("83", 1)
        + ebml("86", b"V_VP9")
        + u("23e383", 500_000_000)
        + ebml("536e", b"Vp9SourceTrack")
        + ebml("22b59c", b"eng")
        + ebml("e0", video),
    )
    tracks = ebml("1654ae6b", track)
    container_tag = ebml(
        "7373",
        ebml("63c0", b"")
        + simple_tag("TITLE", "VP9 Burn Source Title")
        + simple_tag("CUSTOM_SOURCE_NOTE", "PreservedCustomNote"),
    )
    tags = ebml("1254c367", container_tag)
    cluster = u("e7", 0)
    for i in range(3):
        cluster += ebml(
            "a3",
            b"\x81" + (i * 500).to_bytes(2, "big") + b"\x80" + pkt,
        )
    return header + ebml("18538067", info + tracks + tags + ebml("1f43b675", cluster))


def build_vp9_rotated_cropped_burn_source() -> bytes:
    # Coded 64x96, crop left=4, right=4 (visible coded width=56, height=96),
    # projection roll=-90.0 (clockwise 90 deg rotation -> post-rotation 96x56).
    pkt = vp9_frame(8, "420", width=64, height=96)
    header = ebml(
        "1a45dfa3",
        ebml("4282", b"matroska") + u("4287", 4) + u("4285", 2),
    )
    info = ebml(
        "1549a966",
        u("2ad7b1", 1_000_000)
        + ebml("4d80", b"FVidRotatedMux")
        + ebml("5741", b"FVidRotatedBurnSynth"),
    )
    projection = ebml(
        "7670",
        u("7671", 0) + f64_elem("7675", -90.0),
    )
    video = (
        u("b0", 64)
        + u("ba", 96)
        + u("54cc", 4)  # PixelCropLeft = 4
        + u("54dd", 4)  # PixelCropRight = 4
        + projection
    )
    track = ebml(
        "ae",
        u("d7", 1)
        + u("73c5", 1)
        + u("83", 1)
        + ebml("86", b"V_VP9")
        + u("23e383", 500_000_000)
        + ebml("e0", video),
    )
    tracks = ebml("1654ae6b", track)
    cluster = u("e7", 0)
    for i in range(2):
        cluster += ebml(
            "a3",
            b"\x81" + (i * 500).to_bytes(2, "big") + b"\x80" + pkt,
        )
    return header + ebml("18538067", info + tracks + ebml("1f43b675", cluster))


def build_srt() -> bytes:
    return (
        "1\n"
        "00:00:00,500 --> 00:00:01,000\n"
        "FVid\n"
    ).encode("utf-8")


def make_y4m(width: int, height: int, luma_values: list[int]) -> bytes:
    header = f"YUV4MPEG2 W{width} H{height} F2:1 Ip A1:1 C420\n".encode("ascii")
    chroma_size = (width // 2) * (height // 2)
    out = bytearray(header)
    for y_val in luma_values:
        out.extend(b"FRAME\n")
        out.extend(bytes([y_val]) * (width * height))
        out.extend(bytes([128]) * (2 * chroma_size))
    return bytes(out)


def generate_all(dest_dir: Path = OUT) -> dict[str, bytes]:
    dest_dir.mkdir(parents=True, exist_ok=True)
    files = {
        "vp9-burn-source.webm": build_vp9_burn_source(),
        "vp9-rotated-cropped-burn.mkv": build_vp9_rotated_cropped_burn_source(),
        "burn-cues.srt": build_srt(),
        "xfade-main-4x4.y4m": make_y4m(4, 4, [24, 48, 72]),
        "xfade-other-4x4.y4m": make_y4m(4, 4, [160, 192, 224]),
    }
    manifest = {
        "generator": "scripts/generate_subtitle_burn_matroska_xfade_metadata_regressions.py",
        "batch_id": "FVID-0239",
        "files": {
            name: {
                "bytes": len(payload),
                "sha256": hashlib.sha256(payload).hexdigest(),
            }
            for name, payload in files.items()
        },
    }
    manifest_bytes = (json.dumps(manifest, indent=2, sort_keys=True) + "\n").encode("utf-8")
    files["manifest.json"] = manifest_bytes
    for name, payload in files.items():
        (dest_dir / name).write_bytes(payload)
    return files


if __name__ == "__main__":
    generate_all()
