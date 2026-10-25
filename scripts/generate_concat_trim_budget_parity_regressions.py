#!/usr/bin/env python3
"""Generate synthetic WAVE, Ogg Opus, and MP4 AAC fixtures for concat and trim packet-byte budget and error parity regressions (FVID-0257)."""

from __future__ import annotations

import hashlib
import json
from pathlib import Path
import struct


ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / "tests/fixtures/playback-errors/concat-trim-budget-parity"


def _box(fourcc: bytes, payload: bytes) -> bytes:
    return struct.pack(">I4s", 8 + len(payload), fourcc) + payload


def _ogg_crc_table() -> list[int]:
    table = [0] * 256
    for index in range(256):
        word = index << 24
        for _ in range(8):
            if word & 0x8000_0000:
                word = ((word << 1) ^ 0x04C1_1DB7) & 0xFFFF_FFFF
            else:
                word = (word << 1) & 0xFFFF_FFFF
        table[index] = word
    return table


_OGG_TABLE = _ogg_crc_table()


def _ogg_crc(data: bytes) -> int:
    crc = 0
    for byte in data:
        crc = ((crc << 8) & 0xFFFF_FFFF) ^ _OGG_TABLE[((crc >> 24) ^ byte) & 0xFF]
    return crc


def _ogg_page(serial: int, number: int, granule: int, flags: int, packets: list[bytes]) -> bytes:
    table = bytearray()
    body = bytearray()
    for packet in packets:
        body.extend(packet)
        rest = len(packet)
        while rest >= 255:
            table.append(255)
            rest -= 255
        table.append(rest)
    header_prefix = struct.pack("<4sBBqII", b"OggS", 0, flags, granule, serial, number)
    unsealed = header_prefix + struct.pack("<IB", 0, len(table)) + bytes(table) + bytes(body)
    crc = _ogg_crc(unsealed)
    return header_prefix + struct.pack("<IB", crc, len(table)) + bytes(table) + bytes(body)


def build_wave_stereo_s16_bytes() -> bytes:
    # 48 kHz stereo 16-bit PCM (frame_bytes = 4), 48 sample frames = 1 ms = 192 bytes
    pcm = struct.pack("<hh", 256, -256) * 48
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


def build_wave_stereo_f32_bytes() -> bytes:
    # 48 kHz stereo 32-bit float PCM (frame_bytes = 8), 48 sample frames = 1 ms = 384 bytes
    pcm = struct.pack("<ff", 0.125, -0.125) * 48
    fmt = struct.pack("<HHIIHH", 3, 2, 48000, 384000, 8, 32)
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


def build_ogg_opus_mono_bytes() -> bytes:
    # 48 kHz mono Ogg Opus with 6-byte encoded packet (960 samples = 20 ms)
    opus_head = b"OpusHead" + struct.pack("<BBHIhB", 1, 1, 0, 48000, 0, 0)
    opus_tags = b"OpusTags" + struct.pack("<I", 4) + b"fvid" + struct.pack("<I", 0)
    opus_pkt = bytes([0xF8, 0xFF, 0xFE, 0x00, 0x00, 0x00])
    return (
        _ogg_page(1, 0, 0, 0x02, [opus_head])
        + _ogg_page(1, 1, 0, 0x00, [opus_tags])
        + _ogg_page(1, 2, 960, 0x04, [opus_pkt])
    )


def build_mp4_aac_mono_bytes() -> bytes:
    # 48 kHz mono AAC-LC MP4 with two 4-byte silent SCE samples (1024 samples each):
    # id=0 (3b), tag=0 (4b), gain=0 (8b), ics_reserved=0 (1b), seq=0 (2b), shape=0 (1b),
    # max_sfb=0 (6b), pred=0 (1b), pulse=0 (1b), tns=0 (1b), gain_ctrl=0 (1b), id_end=7 (3b) -> 32 bits
    sample = bytes([0x00, 0x00, 0x00, 0x07])
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
        + bytes([0x03, 0x80, 0x80, 0x80, 0x22, 0x00, 0x00, 0x00])
        + bytes([0x04, 0x80, 0x80, 0x80, 0x14, 0x40, 0x15, 0x00, 0x18, 0x00, 0x00, 0x00, 0xFA, 0x00, 0x00, 0x00, 0xFA, 0x00])
        + bytes([0x05, 0x80, 0x80, 0x80, len(asc)])
        + asc
        + bytes([0x06, 0x80, 0x80, 0x80, 0x01, 0x02])
    )
    esds = _box(b"esds", esds_body)
    mp4a = _box(
        b"mp4a",
        b"\x00" * 6
        + struct.pack(">H", 1)
        + b"\x00" * 8
        + struct.pack(">HHHH", 1, 16, 0, 0)
        + struct.pack(">I", 48000 << 16)
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


def generate(out_dir: Path = OUT) -> dict[str, object]:
    out_dir.mkdir(parents=True, exist_ok=True)
    specs = [
        (
            "wave-stereo-s16-48khz.wav",
            build_wave_stereo_s16_bytes(),
            "48 kHz stereo 16-bit PCM WAVE (frame_bytes = 4) for sample-domain concat and trim budget parity",
        ),
        (
            "wave-stereo-f32-48khz.wav",
            build_wave_stereo_f32_bytes(),
            "48 kHz stereo 32-bit float PCM WAVE (frame_bytes = 8) for mixed-precision sample-domain concat budget parity",
        ),
        (
            "ogg-opus-mono-48khz.ogg",
            build_ogg_opus_mono_bytes(),
            "48 kHz mono Ogg Opus (4-byte encoded packet) for concat and trim budget and error parity",
        ),
        (
            "mp4-aac-mono-48khz.m4a",
            build_mp4_aac_mono_bytes(),
            "48 kHz mono AAC-LC MP4 (6-byte encoded packets) for mixed-codec sample-domain concat budget parity",
        ),
    ]
    manifest_entries = []
    for name, data, purpose in specs:
        path = out_dir / name
        path.write_bytes(data)
        manifest_entries.append(
            {
                "name": name,
                "bytes": len(data),
                "sha256": hashlib.sha256(data).hexdigest(),
                "purpose": purpose,
            }
        )
    manifest = {
        "suite": "concat-trim-budget-parity",
        "improvement_id": "FVID-0257",
        "fixtures": manifest_entries,
    }
    (out_dir / "manifest.json").write_text(
        json.dumps(manifest, indent=2) + "\n", encoding="utf-8"
    )
    return manifest


if __name__ == "__main__":
    generate()
