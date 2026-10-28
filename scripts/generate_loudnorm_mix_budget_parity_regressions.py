#!/usr/bin/env python3
"""Generate synthetic WAVE, ADTS AAC, and MP4 AAC fixtures for loudnorm, loudness, and audio mix/merge budget and stream-selection parity regressions (FVID-0255)."""

from __future__ import annotations

import hashlib
import json
from pathlib import Path
import struct


ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / "tests/fixtures/playback-errors/loudnorm-mix-budget-parity"


def _box(fourcc: bytes, payload: bytes) -> bytes:
    return struct.pack(">I4s", 8 + len(payload), fourcc) + payload


def build_wave_stereo_s16_bytes() -> bytes:
    # 48 kHz stereo 16-bit PCM (frame_bytes = 4), 4 sample frames = 16 bytes
    pcm = struct.pack("<hhhhhhhh", 256, -256, 512, -512, 1024, -1024, 256, -256)
    fmt = struct.pack("<HHIIHH", 1, 2, 48000, 192000, 4, 16)
    riff_body = (
        b"WAVE"
        + b"fmt "
        + struct.pack("<I", len(fmt))
        + fmt
        + b"data"
        + struct.pack("<I", len(pcm))
        + pcm
    )
    return b"RIFF" + struct.pack("<I", len(riff_body)) + riff_body


def build_adts_mono_bytes() -> bytes:
    # 48 kHz mono AAC-LC ADTS frame (7-byte header + 6-byte silent SCE payload = 13 bytes)
    payload = bytes([0x21, 0x10, 0x04, 0x60, 0x8C, 0x1C])
    frame_len = 7 + len(payload)
    header = bytearray(7)
    header[0] = 0xFF
    header[1] = 0xF1
    header[2] = (1 << 6) | (3 << 2) | (0 << 1) | ((1 >> 2) & 0x01)
    header[3] = ((1 & 0x03) << 6) | ((frame_len >> 11) & 0x03)
    header[4] = (frame_len >> 3) & 0xFF
    header[5] = ((frame_len & 0x07) << 5) | 0x1F
    header[6] = 0xFC
    return bytes(header) + payload


def build_mp4_aac_mono_bytes() -> bytes:
    # Minimal 48 kHz mono AAC-LC MP4 with two 6-byte samples (1024 samples each)
    sample = bytes([0x21, 0x10, 0x04, 0x60, 0x8C, 0x1C])
    samples = [sample, sample]
    mdat_payload = b"".join(samples)

    ftyp = _box(b"ftyp", b"isom\x00\x00\x02\x00isomiso2mp41")
    mdat_offset = len(ftyp) + 8
    mdat = _box(b"mdat", mdat_payload)

    mvhd = _box(
        b"mvhd",
        struct.pack(">IIIIIIHH", 0, 0, 0, 48000, 2048, 0x00010000, 0x0100, 0)
        + b"\x00" * 10
        + struct.pack(">9I", 0x00010000, 0, 0, 0, 0x00010000, 0, 0, 0, 0x40000000)
        + b"\x00" * 24
        + struct.pack(">I", 2),
    )
    tkhd = _box(
        b"tkhd",
        struct.pack(">IIIIII", 0x00000003, 0, 0, 1, 0, 2048)
        + b"\x00" * 8
        + struct.pack(">HHHH", 0, 0, 0x0100, 0)
        + struct.pack(">9I", 0x00010000, 0, 0, 0, 0x00010000, 0, 0, 0, 0x40000000)
        + struct.pack(">II", 0, 0),
    )
    mdhd = _box(
        b"mdhd",
        struct.pack(">IIIIIIHH", 0, 0, 0, 48000, 2048, 0x55C40000, 0, 0)[:24],
    )
    hdlr = _box(
        b"hdlr",
        struct.pack(">II4s12s", 0, 0, b"soun", b"\x00" * 12) + b"SoundHandler\x00",
    )
    smhd = _box(b"smhd", struct.pack(">II", 0, 0))
    dref = _box(b"dref", struct.pack(">II", 0, 1) + _box(b"url ", struct.pack(">I", 1)))
    dinf = _box(b"dinf", dref)

    # AudioSpecificConfig for AAC-LC 48 kHz mono: 0x11, 0x88
    asc = bytes([0x11, 0x88])
    esds_body = (
        struct.pack(">I", 0)
        + bytes([0x03, 23, 0x00, 0x01, 0x00])
        + bytes([0x04, 15, 0x40, 0x15, 0x00, 0x00, 0x00, 0x00, 0x01, 0xF4, 0x00, 0x00, 0x01, 0xF4, 0x00])
        + bytes([0x05, len(asc)])
        + asc
        + bytes([0x06, 0x01, 0x02])
    )
    esds = _box(b"esds", esds_body)
    mp4a = _box(
        b"mp4a",
        b"\x00" * 6
        + struct.pack(">HHHIHHHI", 1, 0, 0, 0, 1, 16, 0, 48000 << 16)
        + esds,
    )
    stsd = _box(b"stsd", struct.pack(">II", 0, 1) + mp4a)
    stts = _box(b"stts", struct.pack(">IIII", 0, 1, 2, 1024))
    stsc = _box(b"stsc", struct.pack(">IIIII", 0, 1, 1, 2, 1))
    stsz = _box(b"stsz", struct.pack(">IIIII", 0, 0, 2, len(sample), len(sample)))
    stco = _box(b"stco", struct.pack(">III", 0, 1, mdat_offset))
    stbl = _box(b"stbl", stsd + stts + stsc + stsz + stco)
    minf = _box(b"minf", smhd + dinf + stbl)
    mdia = _box(b"mdia", mdhd + hdlr + minf)
    trak = _box(b"trak", tkhd + mdia)
    moov = _box(b"moov", mvhd + trak)
    return ftyp + mdat + moov


def generate() -> dict[str, bytes]:
    wav = build_wave_stereo_s16_bytes()
    adts = build_adts_mono_bytes()
    mp4 = build_mp4_aac_mono_bytes()

    files = {
        "pcm-48k-stereo.wav": wav,
        "aac-48k-mono.aac": adts,
        "aac-48k-mono.m4a": mp4,
    }
    manifest = {
        "batch_id": "FVID-0255",
        "generator": "scripts/generate_loudnorm_mix_budget_parity_regressions.py",
        "sha256": {
            name: hashlib.sha256(data).hexdigest()
            for name, data in sorted(files.items())
        },
    }
    files["manifest.json"] = (
        json.dumps(manifest, indent=2, sort_keys=True) + "\n"
    ).encode("utf-8")
    return files


def main() -> None:
    OUT.mkdir(parents=True, exist_ok=True)
    for name, data in generate().items():
        if name == "manifest.json":
            (OUT / name).write_bytes(data)


if __name__ == "__main__":
    main()
