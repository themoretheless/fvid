#!/usr/bin/env python3
"""Generate synthetic SRT, ASS, Matroska subtitle, Y4M, and Matroska FFV1 fixtures for convert-subtitles, burn-subtitles, xfade, and transcode-lossless packet-budget, memory-budget, and stream-selection parity regressions (FVID-0251)."""

from __future__ import annotations

import hashlib
import json
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / "tests/fixtures/playback-errors/subtitle-convert-burn-budget-parity"
FIXTURES = ROOT / "tests/fixtures/playback-errors"


def ebml_vint_len(length: int) -> bytes:
    if length < 0x7F:
        return bytes([0x80 | length])
    if length < 0x3FFF:
        return bytes([0x40 | (length >> 8), length & 0xFF])
    return bytes(
        [
            0x20 | (length >> 16),
            (length >> 8) & 0xFF,
            length & 0xFF,
        ]
    )


def ebml_atom(elem_id: bytes, payload: bytes) -> bytes:
    return elem_id + ebml_vint_len(len(payload)) + payload


def build_plain_srt() -> bytes:
    return (
        b"1\n"
        b"00:00:00,000 --> 00:00:01,000\n"
        b"Hello Synthetic Subtitle\n\n"
        b"2\n"
        b"00:00:01,000 --> 00:00:02,000\n"
        b"Second Cue\n"
    )


def build_plain_ass() -> bytes:
    return (
        b"[Script Info]\n"
        b"ScriptType: v4.00+\n"
        b"PlayResX: 384\n"
        b"PlayResY: 288\n"
        b"[V4+ Styles]\n"
        b"Format: Name, Fontname, Fontsize, PrimaryColour, SecondaryColour, OutlineColour, BackColour, Bold, Italic, Underline, StrikeOut, ScaleX, ScaleY, Spacing, Angle, BorderStyle, Outline, Shadow, Alignment, MarginL, MarginR, MarginV, Encoding\n"
        b"Style: Default,Arial,16,&H00FFFFFF,&H000000FF,&H00000000,&H00000000,0,0,0,0,100,100,0,0,1,1,0,2,10,10,10,1\n"
        b"[Events]\n"
        b"Format: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\n"
        b"Dialogue: 0,0:00:00.00,0:00:01.00,Default,,0,0,0,,Hello ASS Cue\n"
    )


def build_matroska_subrip_with_audio_packet() -> bytes:
    ebml_header = ebml_atom(
        b"\x1a\x45\xdf\xa3",
        ebml_atom(b"\x42\x82", b"matroska")
        + ebml_atom(b"\x42\x87", b"\x04")
        + ebml_atom(b"\x42\x85", b"\x02"),
    )
    info = ebml_atom(
        b"\x15\x49\xa9\x66",
        ebml_atom(b"\x2a\xd7\xb1", (1_000_000).to_bytes(4, "big"))
        + ebml_atom(b"\x4d\x80", b"FVid")
        + ebml_atom(b"\x57\x41", b"FVid"),
    )
    audio_settings = ebml_atom(
        b"\xe1",
        ebml_atom(b"\xb5", bytes.fromhex("40e7700000000000"))  # 48000.0
        + ebml_atom(b"\x9f", b"\x01")
        + ebml_atom(b"\x62\x64", b"\x10"),
    )
    audio_track = ebml_atom(
        b"\xae",
        ebml_atom(b"\xd7", b"\x01")
        + ebml_atom(b"\x73\xc5", b"\x01")
        + ebml_atom(b"\x83", b"\x02")
        + ebml_atom(b"\x86", b"A_PCM/INT/LIT")
        + audio_settings,
    )
    sub_track = ebml_atom(
        b"\xae",
        ebml_atom(b"\xd7", b"\x02")
        + ebml_atom(b"\x73\xc5", b"\x02")
        + ebml_atom(b"\x83", b"\x11")
        + ebml_atom(b"\x86", b"S_TEXT/UTF8"),
    )
    tracks = ebml_atom(b"\x16\x54\xae\x6b", audio_track + sub_track)
    # Audio block (track 1): 128 bytes of silence
    audio_block = ebml_atom(
        b"\xa3",
        b"\x81\x00\x00\x80" + (b"\x00" * 128),
    )
    # Subtitle block group (track 2): 16-byte cue "Matroska SubRip!", duration 1000 ms
    sub_block = ebml_atom(
        b"\xa1",
        b"\x82\x00\x00\x00Matroska SubRip!",
    )
    sub_group = ebml_atom(
        b"\xa0",
        sub_block + ebml_atom(b"\x9b", (1000).to_bytes(2, "big")),
    )
    cluster = ebml_atom(
        b"\x1f\x43\xb6\x75",
        ebml_atom(b"\xe7", b"\x00") + audio_block + sub_group,
    )
    seg_payload = info + tracks + cluster
    segment = ebml_atom(b"\x18\x53\x80\x67", seg_payload)
    return ebml_header + segment


def build_y4m_pair() -> tuple[bytes, bytes]:
    header = b"YUV4MPEG2 W96 H64 F2:1 Ip A1:1 C420\n"
    y0 = bytes([32] * (96 * 64))
    uv0 = bytes([128] * (2 * 48 * 32))
    y1 = bytes([96] * (96 * 64))
    uv1 = bytes([128] * (2 * 48 * 32))
    frame0 = b"FRAME\n" + y0 + uv0
    frame1 = b"FRAME\n" + y1 + uv1
    return (header + frame0 + frame0, header + frame1 + frame1)


def generate() -> dict[str, bytes]:
    y4m_a, y4m_b = build_y4m_pair()
    files = {
        "sample.srt": build_plain_srt(),
        "sample.ass": build_plain_ass(),
        "muxed-audio-and-subrip.mkv": build_matroska_subrip_with_audio_packet(),
        "burn-primary.y4m": y4m_a,
        "burn-secondary.y4m": y4m_b,
        "burn-source-ffv1.mkv": (FIXTURES / "ffv1-six-frames.mkv").read_bytes(),
    }
    manifest = {
        "batch_id": "FVID-0251",
        "sha256": {name: hashlib.sha256(data).hexdigest() for name, data in files.items()},
    }
    files["manifest.json"] = (json.dumps(manifest, indent=2, sort_keys=True) + "\n").encode("utf-8")
    return files


def main() -> None:
    OUT.mkdir(parents=True, exist_ok=True)
    for name, data in generate().items():
        (OUT / name).write_bytes(data)


if __name__ == "__main__":
    main()
