#!/usr/bin/env python3
"""Generate deterministic synthetic fixtures for Batch 253 (FVID-0260):
xfade, burn-subtitles, and convert-subtitles packet-byte / packet-work budget parity.
"""

from __future__ import annotations

import pathlib
import struct


def ebml_vint(value: int, width: int | None = None) -> bytes:
    if width is None:
        for w in range(1, 9):
            limit = (1 << (7 * w)) - 2
            if value <= limit:
                width = w
                break
        else:
            raise ValueError("vint overflow")
    marker = 1 << (8 - width)
    out = bytearray(width)
    v = value
    for i in range(width - 1, -1, -1):
        out[i] = v & 0xFF
        v >>= 8
    out[0] |= marker
    return bytes(out)


def ebml_id(raw_hex: str) -> bytes:
    return bytes.fromhex(raw_hex)


def ebml_elem(raw_id: str, payload: bytes) -> bytes:
    return ebml_id(raw_id) + ebml_vint(len(payload)) + payload


def ebml_uint(raw_id: str, value: int) -> bytes:
    if value == 0:
        b = b"\x00"
    else:
        n = (value.bit_length() + 7) // 8
        b = value.to_bytes(n, "big")
    return ebml_elem(raw_id, b)


def ebml_str(raw_id: str, text: str) -> bytes:
    return ebml_elem(raw_id, text.encode("utf-8"))


def build_textured_y4m(width: int = 96, height: int = 64, frames: int = 3, shift: int = 0) -> bytes:
    header = f"YUV4MPEG2 W{width} H{height} F2:1 Ip A1:1 C420\n".encode("ascii")
    out = bytearray(header)
    cw = width // 2
    ch = height // 2
    state = (0x9E3779B9 ^ (shift * 0x85EBCA6B)) & 0xFFFFFFFF
    for _ in range(frames):
        out.extend(b"FRAME\n")
        for _ in range(height * width + 2 * ch * cw):
            state ^= (state << 13) & 0xFFFFFFFF
            state ^= (state >> 17) & 0xFFFFFFFF
            state ^= (state << 5) & 0xFFFFFFFF
            out.append(state & 0xFF)
    return bytes(out)


def build_two_cue_srt() -> bytes:
    # Cue 0 body is 40 bytes ("SubRip cue zero payload with forty bytes"),
    # so formatted ASS event "0,0,Default,,0,0,0,,SubRip cue zero payload with forty bytes" is 60 bytes.
    # Cue 1 body is 8 bytes ("Cue one!").
    text = (
        "1\n"
        "00:00:00,000 --> 00:00:01,000\n"
        "SubRip cue zero payload with forty bytes\n"
        "\n"
        "2\n"
        "00:00:01,000 --> 00:00:02,000\n"
        "Cue one!\n"
    )
    return text.encode("utf-8")


def build_burn_srt() -> bytes:
    text = (
        "1\n"
        "00:00:00,000 --> 00:00:01,500\n"
        "Burned subtitle text for packet limit parity\n"
    )
    return text.encode("utf-8")


def build_two_cue_subrip_mkv() -> bytes:
    # Matroska with a single S_TEXT/UTF8 track (track number 1, kind 17) and two BlockGroup packets:
    # Packet 0: pts=0ms, duration=500ms, payload = "SubRip cue zero payload with forty bytes" (40 bytes raw, 60 bytes formatted ASS event).
    # Packet 1: pts=500ms, duration=500ms, payload = "Cue one!" (8 bytes raw, 28 bytes formatted ASS event).
    ebml_header = ebml_elem(
        "1A45DFA3",
        ebml_uint("4286", 1)
        + ebml_uint("42F7", 1)
        + ebml_uint("42F2", 4)
        + ebml_uint("42F3", 8)
        + ebml_str("4282", "matroska")
        + ebml_uint("4287", 2)
        + ebml_uint("4285", 2),
    )
    info = ebml_elem(
        "1549A966",
        ebml_uint("2AD7B1", 1_000_000)
        + ebml_str("4D80", "FVidTest")
        + ebml_str("5741", "FVidTest"),
    )
    track_entry = ebml_elem(
        "AE",
        ebml_uint("D7", 1)
        + ebml_uint("73C5", 1)
        + ebml_uint("83", 17)
        + ebml_str("86", "S_TEXT/UTF8")
        + ebml_str("22B59C", "eng"),
    )
    tracks = ebml_elem("1654AE6B", track_entry)

    def make_block_group(rel_ts_ms: int, duration_ms: int, payload: bytes) -> bytes:
        block_header = ebml_vint(1, 1) + struct.pack(">hB", rel_ts_ms, 0)
        block = ebml_elem("A1", block_header + payload)
        dur = ebml_uint("9B", duration_ms)
        return ebml_elem("A0", block + dur)

    bg0 = make_block_group(0, 500, b"SubRip cue zero payload with forty bytes")
    bg1 = make_block_group(500, 500, b"Cue one!")
    cluster = ebml_elem("1F43B675", ebml_uint("E7", 0) + bg0 + bg1)

    segment_payload = info + tracks + cluster
    segment = ebml_id("18538067") + ebml_vint(len(segment_payload), 8) + segment_payload
    return ebml_header + segment


def generate_all(dest_dir: pathlib.Path | None = None) -> dict[str, pathlib.Path]:
    if dest_dir is None:
        dest_dir = (
            pathlib.Path(__file__).resolve().parent.parent
            / "tests"
            / "fixtures"
            / "playback-errors"
            / "xfade-subtitle-budget-parity"
        )
    dest_dir.mkdir(parents=True, exist_ok=True)

    files = {
        "xfade-primary.y4m": build_textured_y4m(96, 64, 3, 0),
        "xfade-secondary.y4m": build_textured_y4m(96, 64, 3, 7),
        "two-cues.srt": build_two_cue_srt(),
        "burn.srt": build_burn_srt(),
        "two-cues-subrip.mkv": build_two_cue_subrip_mkv(),
    }
    paths: dict[str, pathlib.Path] = {}
    for name, content in files.items():
        p = dest_dir / name
        p.write_bytes(content)
        paths[name] = p
    return paths


if __name__ == "__main__":
    generate_all()
