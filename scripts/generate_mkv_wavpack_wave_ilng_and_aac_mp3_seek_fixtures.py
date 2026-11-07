#!/usr/bin/env python3
"""Generate synthetic fixtures for Matroska WavPack probing, WAVE ILNG metadata round-trip, and ADTS/MP3 sub-sample seek_time."""

from __future__ import annotations

import pathlib
import struct


ROOT = pathlib.Path(__file__).resolve().parents[1]
OUT_DIR = ROOT / "tests" / "fixtures" / "playback-errors" / "mkv-wavpack-wave-ilng-and-aac-mp3-seek"


def ebml_id(raw: int) -> bytes:
    out = []
    while raw > 0:
        out.append(raw & 0xFF)
        raw >>= 8
    return bytes(reversed(out))


def ebml_size(length: int) -> bytes:
    for width in range(1, 9):
        limit = (1 << (7 * width)) - 2
        if length <= limit:
            val = length | (1 << (7 * width))
            return val.to_bytes(width, "big")
    raise ValueError("size too large")


def ebml_elem(raw_id: int, payload: bytes) -> bytes:
    return ebml_id(raw_id) + ebml_size(len(payload)) + payload


def ebml_uint(raw_id: int, val: int) -> bytes:
    if val == 0:
        return ebml_elem(raw_id, b"\x00")
    out = []
    while val > 0:
        out.append(val & 0xFF)
        val >>= 8
    return ebml_elem(raw_id, bytes(reversed(out)))


def ebml_float64(raw_id: int, val: float) -> bytes:
    return ebml_elem(raw_id, struct.pack(">d", val))


def ebml_str(raw_id: int, text: str) -> bytes:
    return ebml_elem(raw_id, text.encode("utf-8"))


def make_mkv_with_wavpack() -> bytes:
    ebml_head = ebml_elem(
        0x1A45DFA3,
        ebml_uint(0x4286, 1)
        + ebml_uint(0x42F7, 1)
        + ebml_uint(0x42F2, 4)
        + ebml_uint(0x42F3, 8)
        + ebml_str(0x4282, "matroska")
        + ebml_uint(0x4287, 4)
        + ebml_uint(0x4285, 2),
    )
    info = ebml_elem(
        0x1549A966,
        ebml_uint(0x2AD7B1, 1_000_000) + ebml_float64(0x4489, 100.0),
    )
    audio_settings = ebml_elem(
        0xE1,
        ebml_float64(0xB5, 48000.0) + ebml_uint(0x9F, 2) + ebml_uint(0x6264, 16),
    )
    track = ebml_elem(
        0xAE,
        ebml_uint(0xD7, 1)
        + ebml_uint(0x73C5, 1)
        + ebml_uint(0x83, 2)  # Audio
        + ebml_str(0x86, "A_WAVPACK4")
        + audio_settings,
    )
    tracks = ebml_elem(0x1654AE6B, track)
    block = bytes([0x81, 0x00, 0x00, 0x80, 0x00, 0x00])
    cluster = ebml_elem(
        0x1F43B675,
        ebml_uint(0xE7, 0) + ebml_elem(0xA3, block),
    )
    segment = ebml_elem(0x18538067, info + tracks + cluster)
    return ebml_head + segment


def riff_chunk(fourcc: bytes, payload: bytes) -> bytes:
    pad = b"\x00" if (len(payload) & 1) else b""
    return fourcc + struct.pack("<I", len(payload)) + payload + pad


def make_wave_with_ilng() -> bytes:
    # 1 ch, 8000 Hz, 16-bit PCM, 4 samples
    fmt = struct.pack("<HHIIHH", 1, 1, 8000, 16000, 2, 16)
    data = struct.pack("<hhhh", 0, 1000, -1000, 0)
    info = riff_chunk(b"INAM", b"Test Wave\x00") + riff_chunk(b"ILNG", b"eng\x00")
    list_info = riff_chunk(b"LIST", b"INFO" + info)
    body = b"WAVE" + riff_chunk(b"fmt ", fmt) + list_info + riff_chunk(b"data", data)
    return riff_chunk(b"RIFF", body)


def make_adts_1frame() -> bytes:
    # ADTS header (7 bytes) + 1 byte raw_data_block (ID_END = 0xE0) = 8 bytes total
    # 48000 Hz (sf_index=3), stereo (channel_configuration=2), AAC-LC (profile_minus_1=1)
    frame_length = 8
    b0 = 0xFF
    b1 = 0xF1
    b2 = (1 << 6) | (3 << 2) | 0  # profile=1 (AAC-LC), sf_idx=3 (48kHz)
    b3 = (2 << 6) | ((frame_length >> 11) & 0x03)
    b4 = (frame_length >> 3) & 0xFF
    b5 = ((frame_length & 0x07) << 5) | 0x1F
    b6 = 0xFC
    return bytes([b0, b1, b2, b3, b4, b5, b6, 0xE0])


def make_mp2_1frame() -> bytes:
    # MPEG-1 Layer II, 32 kbps (bitrate_idx=1), 48000 Hz (sr_idx=1), mono (mode=3)
    # Frame size = 144 * 32000 // 48000 = 96 bytes
    header = bytes([0xFF, 0xFD, 0x14, 0xC0])
    return header + (b"\x00" * 92)


def main() -> None:
    OUT_DIR.mkdir(parents=True, exist_ok=True)
    (OUT_DIR / "wavpack_track.mkv").write_bytes(make_mkv_with_wavpack())
    (OUT_DIR / "wave_with_ilng.wav").write_bytes(make_wave_with_ilng())
    (OUT_DIR / "tiny_48k.aac").write_bytes(make_adts_1frame())
    (OUT_DIR / "tiny_48k.mp2").write_bytes(make_mp2_1frame())


if __name__ == "__main__":
    main()
