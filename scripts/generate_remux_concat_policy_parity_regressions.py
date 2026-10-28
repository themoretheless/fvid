#!/usr/bin/env python3
"""Generate synthetic media fixtures for remux/concat option and error parity regressions."""

from __future__ import annotations

import json
import struct
from pathlib import Path


def _mp4_box(fourcc: bytes, payload: bytes) -> bytes:
    return struct.pack(">I4s", 8 + len(payload), fourcc) + payload


def _mp4_full_box(fourcc: bytes, version: int, flags: int, payload: bytes) -> bytes:
    header = struct.pack(">I", ((version & 0xFF) << 24) | (flags & 0xFFFFFF))
    return _mp4_box(fourcc, header + payload)


def build_mp4_aac_mono_48khz() -> bytes:
    """Build a synthetic 1-track MP4 with 2 frames of 48 kHz mono AAC-LC (4 bytes each)."""
    sample = bytes([0x00, 0x00, 0x00, 0x07])
    mdat_payload = sample * 2
    ftyp = _mp4_box(b"ftyp", b"isom\x00\x00\x02\x00isomiso2mp41")
    mdat = _mp4_box(b"mdat", mdat_payload)
    mdat_data_offset = len(ftyp) + 8

    mvhd = _mp4_box(
        b"mvhd",
        struct.pack(">IIIIIIHH", 0, 0, 0, 48000, 2048, 0x00010000, 0x0100, 0)
        + b"\x00" * 10
        + struct.pack(">9I", 0x00010000, 0, 0, 0, 0x00010000, 0, 0, 0, 0x40000000)
        + b"\x00" * 24
        + struct.pack(">I", 2),
    )
    tkhd = _mp4_box(
        b"tkhd",
        struct.pack(">IIIIII", 0x00000003, 0, 0, 1, 0, 2048)
        + b"\x00" * 8
        + struct.pack(">HHHH", 0, 0, 0x0100, 0)
        + struct.pack(">9I", 0x00010000, 0, 0, 0, 0x00010000, 0, 0, 0, 0x40000000)
        + struct.pack(">II", 0, 0),
    )
    mdhd = _mp4_box(
        b"mdhd",
        struct.pack(">IIIIIIHH", 0, 0, 0, 48000, 2048, 0x55C40000, 0, 0)[:24],
    )
    hdlr = _mp4_box(
        b"hdlr",
        struct.pack(">II4s12s", 0, 0, b"soun", b"\x00" * 12) + b"SoundHandler\x00",
    )
    smhd = _mp4_box(b"smhd", struct.pack(">II", 0, 0))
    dref = _mp4_box(
        b"dref",
        struct.pack(">II", 0, 1) + _mp4_box(b"url ", struct.pack(">I", 1)),
    )
    dinf = _mp4_box(b"dinf", dref)

    asc = bytes([0x11, 0x88])
    esds_body = (
        struct.pack(">I", 0)
        + bytes([0x03, 0x80, 0x80, 0x80, 0x22, 0x00, 0x00, 0x00])
        + bytes(
            [
                0x04,
                0x80,
                0x80,
                0x80,
                0x14,
                0x40,
                0x15,
                0x00,
                0x18,
                0x00,
                0x00,
                0x00,
                0xFA,
                0x00,
                0x00,
                0x00,
                0xFA,
                0x00,
            ]
        )
        + bytes([0x05, 0x80, 0x80, 0x80, len(asc)])
        + asc
        + bytes([0x06, 0x80, 0x80, 0x80, 0x01, 0x02])
    )
    esds = _mp4_box(b"esds", esds_body)
    mp4a = _mp4_box(
        b"mp4a",
        b"\x00" * 6
        + struct.pack(">H", 1)
        + b"\x00" * 8
        + struct.pack(">HHHH", 1, 16, 0, 0)
        + struct.pack(">I", 48000 << 16)
        + esds,
    )
    stsd = _mp4_full_box(b"stsd", 0, 0, struct.pack(">I", 1) + mp4a)
    stts = _mp4_full_box(b"stts", 0, 0, struct.pack(">III", 1, 2, 1024))
    stsc = _mp4_full_box(b"stsc", 0, 0, struct.pack(">IIII", 1, 1, 2, 1))
    stsz = _mp4_full_box(b"stsz", 0, 0, struct.pack(">IIII", 0, 2, len(sample), len(sample)))
    stco = _mp4_full_box(b"stco", 0, 0, struct.pack(">II", 1, mdat_data_offset))
    stbl = _mp4_box(b"stbl", stsd + stts + stsc + stsz + stco)
    minf = _mp4_box(b"minf", smhd + dinf + stbl)
    mdia = _mp4_box(b"mdia", mdhd + hdlr + minf)
    trak = _mp4_box(b"trak", tkhd + mdia)
    moov = _mp4_box(b"moov", mvhd + trak)
    return ftyp + mdat + moov


def _ebml_id(value: int) -> bytes:
    if value < 0x100:
        return bytes([value])
    if value < 0x10000:
        return struct.pack(">H", value)
    if value < 0x1000000:
        return value.to_bytes(3, "big")
    return struct.pack(">I", value)


def _ebml_vint(length: int) -> bytes:
    if length < 0x7F:
        return bytes([0x80 | length])
    if length < 0x3FFF:
        return struct.pack(">H", 0x4000 | length)
    return struct.pack(">I", 0x10000000 | length)


def _ebml_el(tag: int, body: bytes) -> bytes:
    return _ebml_id(tag) + _ebml_vint(len(body)) + body


def build_matroska_pcm_mono_48khz() -> bytes:
    """Build a minimal valid Matroska A_PCM/FLOAT/IEEE mono 48 kHz fixture with 16-byte packet."""
    ebml_header = _ebml_el(
        0x1A45DFA3,
        _ebml_el(0x4286, b"\x01")
        + _ebml_el(0x42F7, b"\x01")
        + _ebml_el(0x42F2, b"\x04")
        + _ebml_el(0x42F3, b"\x08")
        + _ebml_el(0x4282, b"matroska")
        + _ebml_el(0x4287, b"\x02")
        + _ebml_el(0x4285, b"\x02"),
    )
    info = _ebml_el(
        0x1549A966,
        _ebml_el(0x2AD7B1, struct.pack(">I", 1_000_000))
        + _ebml_el(0x4D80, b"fvid-test")
        + _ebml_el(0x5741, b"fvid-test"),
    )
    audio = _ebml_el(
        0xE1,
        _ebml_el(0xB5, struct.pack(">d", 48000.0))
        + _ebml_el(0x9F, b"\x01")
        + _ebml_el(0x6264, b"\x20"),
    )
    track_entry = _ebml_el(
        0xAE,
        _ebml_el(0xD7, b"\x01")
        + _ebml_el(0x73C5, b"\x01")
        + _ebml_el(0x83, b"\x02")
        + _ebml_el(0x86, b"A_PCM/FLOAT/IEEE")
        + audio,
    )
    tracks = _ebml_el(0x1654AE6B, track_entry)
    pcm_samples = struct.pack("<ffff", 0.0, 0.25, -0.25, 0.0)
    simple_block = _ebml_el(0xA3, b"\x81\x00\x00\x80" + pcm_samples)
    cluster = _ebml_el(0x1F43B675, _ebml_el(0xE7, b"\x00") + simple_block)
    segment = _ebml_el(0x18538067, info + tracks + cluster)
    return ebml_header + segment


def generate_fixtures(output_dir: Path | None = None) -> dict[str, str]:
    root = output_dir or (
        Path(__file__).resolve().parents[1]
        / "tests"
        / "fixtures"
        / "playback-errors"
        / "remux-concat-policy-parity"
    )
    root.mkdir(parents=True, exist_ok=True)
    mp4_path = root / "mp4-aac-mono-48khz.m4a"
    mka_path = root / "matroska-pcm-mono-48khz.mka"
    mp4_path.write_bytes(build_mp4_aac_mono_48khz())
    mka_path.write_bytes(build_matroska_pcm_mono_48khz())
    manifest = {
        "suite": "remux-concat-policy-parity",
        "fixtures": {
            "mp4_aac": mp4_path.name,
            "matroska_pcm": mka_path.name,
        },
    }
    (root / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8")
    return {k: str(root / v) for k, v in manifest["fixtures"].items()}


if __name__ == "__main__":
    generate_fixtures()
