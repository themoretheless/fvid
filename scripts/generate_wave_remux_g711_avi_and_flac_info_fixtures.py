#!/usr/bin/env python3
"""Generate synthetic fixtures for Batch 578 (FVID-0620..0624)."""

from __future__ import annotations

import pathlib
import struct


ROOT = (
    pathlib.Path(__file__).resolve().parent.parent
    / "tests"
    / "fixtures"
    / "playback-errors"
    / "wave-remux-g711-avi-and-flac-info"
)

KSDATAFORMAT_SUBTYPE_TAIL = bytes(
    [0x00, 0x00, 0x10, 0x00, 0x80, 0x00, 0x00, 0xAA, 0x00, 0x38, 0x9B, 0x71]
)


def riff_chunk(tag: bytes, payload: bytes) -> bytes:
    pad = b"\x00" if (len(payload) & 1) else b""
    return tag + struct.pack("<I", len(payload)) + payload + pad


def build_classic_pcm16_stereo_wav() -> bytes:
    # Classic 16-byte fmt: 48 kHz, 2 channels, s16le (implicit mask FL|FR = 0x3)
    fmt = struct.pack("<HHIIHH", 1, 2, 48000, 48000 * 4, 4, 16)
    samples = struct.pack("<hhhh", 1000, -1000, 2000, -2000)
    body = b"WAVE" + riff_chunk(b"fmt ", fmt) + riff_chunk(b"data", samples)
    return riff_chunk(b"RIFF", body)


def build_extensible_directout_pcm16_stereo_wav() -> bytes:
    # WAVEFORMATEXTENSIBLE 40-byte fmt: 48 kHz, 2 channels, s16le, explicit dwChannelMask = 0
    guid = struct.pack("<I", 1) + KSDATAFORMAT_SUBTYPE_TAIL
    fmt = (
        struct.pack("<HHIIHHHHI", 0xFFFE, 2, 48000, 48000 * 4, 4, 16, 22, 16, 0)
        + guid
    )
    fact = struct.pack("<I", 2)
    samples = struct.pack("<hhhh", 3000, -3000, 4000, -4000)
    body = (
        b"WAVE"
        + riff_chunk(b"fmt ", fmt)
        + riff_chunk(b"fact", fact)
        + riff_chunk(b"data", samples)
    )
    return riff_chunk(b"RIFF", body)


def build_extensible_g711_alaw_wav(valid_bits: int, channel_mask: int) -> bytes:
    # WAVEFORMATEXTENSIBLE 40-byte fmt: 8 kHz, 2 channels, A-law (subformat 6)
    guid = struct.pack("<I", 6) + KSDATAFORMAT_SUBTYPE_TAIL
    fmt = (
        struct.pack(
            "<HHIIHHHHI",
            0xFFFE,
            2,
            8000,
            8000 * 2,
            2,
            8,
            22,
            valid_bits,
            channel_mask,
        )
        + guid
    )
    fact = struct.pack("<I", 4)
    # 4 stereo frames (8 A-law bytes)
    samples = bytes([0xD5, 0x55, 0xD5, 0x55, 0xD5, 0x55, 0xD5, 0x55])
    body = (
        b"WAVE"
        + riff_chunk(b"fmt ", fmt)
        + riff_chunk(b"fact", fact)
        + riff_chunk(b"data", samples)
    )
    return riff_chunk(b"RIFF", body)


def build_avi_with_extensible_alaw() -> bytes:
    # Minimal RIFF AVI with one auds stream using WAVEFORMATEXTENSIBLE A-law (subformat 6)
    rate = 8000
    channels = 2
    block_align = 2
    frames = 4
    audio_bytes = bytes([0xD5, 0x55] * frames)

    avih = struct.pack(
        "<14I",
        125_000,  # dwMicroSecPerFrame
        rate * block_align,  # dwMaxBytesPerSec
        0,  # dwPaddingGranularity
        0x10,  # dwFlags (AVIF_HASINDEX)
        1,  # dwTotalFrames
        0,  # dwInitialFrames
        1,  # dwStreams
        len(audio_bytes),  # dwSuggestedBufferSize
        0,  # dwWidth
        0,  # dwHeight
        0,
        0,
        0,
        0,
    )

    strh = struct.pack(
        "<4s4sIIIIIIIIII4h",
        b"auds",
        b"\x00\x00\x00\x00",
        0,  # dwFlags (8)
        0,  # wPriority/wLanguage (12)
        0,  # dwInitialFrames (16)
        1,  # dwScale (20)
        rate,  # dwRate (24)
        0,  # dwStart (28)
        frames,  # dwLength (32)
        len(audio_bytes),  # dwSuggestedBufferSize (36)
        0xFFFFFFFF,  # dwQuality (40)
        block_align,  # dwSampleSize (44)
        0,
        0,
        0,
        0,
    )

    guid = struct.pack("<I", 6) + KSDATAFORMAT_SUBTYPE_TAIL
    strf = (
        struct.pack(
            "<HHIIHHHHI",
            0xFFFE,
            channels,
            rate,
            rate * block_align,
            block_align,
            8,
            22,
            8,
            0x3,  # FL | FR
        )
        + guid
    )

    strl = riff_chunk(b"LIST", b"strl" + riff_chunk(b"strh", strh) + riff_chunk(b"strf", strf))
    hdrl = riff_chunk(b"LIST", b"hdrl" + riff_chunk(b"avih", avih) + strl)

    wb_chunk = riff_chunk(b"00wb", audio_bytes)
    movi = riff_chunk(b"LIST", b"movi" + wb_chunk)

    # idx1 offset 4 (relative to movi 'movi' tag start)
    idx1 = riff_chunk(
        b"idx1",
        struct.pack("<4sIII", b"00wb", 0x10, 4, len(audio_bytes)),
    )

    return riff_chunk(b"RIFF", b"AVI " + hdrl + movi + idx1)


def build_vorbis_comment_block_with_empty_preceding_tags() -> bytes:
    fields = [
        b"TITLE=",
        b"TITLE=Valid Song Title",
        b"DATE=",
        b"YEAR=2026",
    ]
    vendor = b"fvid"
    payload = bytearray()
    payload += struct.pack("<I", len(vendor)) + vendor
    payload += struct.pack("<I", len(fields))
    for field in fields:
        payload += struct.pack("<I", len(field)) + field
    # Metadata block header: last=1, type=4 (VORBIS_COMMENT) -> 0x84, 24-bit big-endian length
    header = bytes([0x84, (len(payload) >> 16) & 0xFF, (len(payload) >> 8) & 0xFF, len(payload) & 0xFF])
    return header + bytes(payload)


def main() -> None:
    ROOT.mkdir(parents=True, exist_ok=True)
    # Fix struct format in build_classic/extensible:
    guid = struct.pack("<I", 1) + KSDATAFORMAT_SUBTYPE_TAIL
    ext_pcm = (
        struct.pack("<HHIIHHHHI", 0xFFFE, 2, 48000, 48000 * 4, 4, 16, 22, 16, 0)
        + guid
    )
    fact = struct.pack("<I", 2)
    samples = struct.pack("<hhhh", 3000, -3000, 4000, -4000)
    ext_pcm_wav = riff_chunk(
        b"RIFF",
        b"WAVE"
        + riff_chunk(b"fmt ", ext_pcm)
        + riff_chunk(b"fact", fact)
        + riff_chunk(b"data", samples),
    )

    (ROOT / "classic_stereo_pcm16.wav").write_bytes(build_classic_pcm16_stereo_wav())
    (ROOT / "extensible_directout_stereo_pcm16.wav").write_bytes(ext_pcm_wav)
    (ROOT / "extensible_directout_alaw.wav").write_bytes(
        build_extensible_g711_alaw_wav(valid_bits=8, channel_mask=0)
    )
    (ROOT / "extensible_alaw_invalid_valid_bits.wav").write_bytes(
        build_extensible_g711_alaw_wav(valid_bits=4, channel_mask=0x3)
    )
    (ROOT / "avi_extensible_alaw.avi").write_bytes(build_avi_with_extensible_alaw())
    (ROOT / "flac_vorbis_comment_empty_and_valid.bin").write_bytes(
        build_vorbis_comment_block_with_empty_preceding_tags()
    )


if __name__ == "__main__":
    main()
