#!/usr/bin/env python3
"""Generate synthetic fixtures for AIFF empty block-aligned SSND, AVI audio seeking, FFV1 411/410 MP4 stats, and MKV bitmap subtitle/data track probing."""

from __future__ import annotations

import pathlib
import struct


ROOT = pathlib.Path(__file__).resolve().parents[1]
OUT_DIR = ROOT / "tests" / "fixtures" / "playback-errors" / "aiff-avi-ffv1-and-mkv-probe"


def encode_80bit_float(rate: int) -> bytes:
    exp = 16383 + 31
    mant = rate
    while mant and (mant & 0x80000000) == 0:
        mant <<= 1
        exp -= 1
    return struct.pack(">HQ", exp, mant << 32)


def make_aiff_block_aligned_zero_frames() -> bytes:
    # COMM chunk: 1 channel, 0 declared sample frames, 16-bit PCM, 8000 Hz
    comm_body = struct.pack(">hIh", 1, 0, 16) + encode_80bit_float(8000)
    comm = b"COMM" + struct.pack(">I", len(comm_body)) + comm_body
    # SSND chunk: offset = 0, block_size = 512, followed by 512 padding bytes
    ssnd_body = struct.pack(">II", 0, 512) + (b"\x00" * 512)
    ssnd = b"SSND" + struct.pack(">I", len(ssnd_body)) + ssnd_body
    form_body = b"AIFF" + comm + ssnd
    return b"FORM" + struct.pack(">I", len(form_body)) + form_body


def riff_chunk(fourcc: bytes, payload: bytes) -> bytes:
    pad = b"\x00" if (len(payload) & 1) else b""
    return fourcc + struct.pack("<I", len(payload)) + payload + pad


def riff_list(list_type: bytes, body: bytes) -> bytes:
    return riff_chunk(b"LIST", list_type + body)


def make_avi_pcm_4frames() -> bytes:
    # 1 channel, 16-bit PCM, 4 Hz, 4 samples [1000, 2000, 3000, 4000]
    sample_rate = 4
    channels = 1
    bits = 16
    block_align = channels * (bits // 8)
    byte_rate = sample_rate * block_align
    samples = struct.pack("<hhhh", 1000, 2000, 3000, 4000)
    num_frames = 4

    avih = struct.pack(
        "<IIIIIIIIIIIIII",
        250_000,  # dwMicroSecPerFrame
        byte_rate,
        0,
        0x10,
        num_frames,
        0,
        1,  # dwStreams
        len(samples),
        0,
        0,
        0,
        0,
        0,
        0,
    )
    strh = struct.pack(
        "<4s4sIHHIIIIIIIIhhhh",
        b"auds",
        b"\x00\x00\x00\x00",
        0,
        0,
        0,
        0,
        1,  # dwScale
        sample_rate,  # dwRate
        0,  # dwStart
        num_frames,  # dwLength
        len(samples),
        0,
        block_align,
        0,
        0,
        0,
        0,
    )
    strf = struct.pack(
        "<HHIIHH",
        1,  # WAVE_FORMAT_PCM
        channels,
        sample_rate,
        byte_rate,
        block_align,
        bits,
    )
    strl = riff_list(b"strl", riff_chunk(b"strh", strh) + riff_chunk(b"strf", strf))
    hdrl = riff_list(b"hdrl", riff_chunk(b"avih", avih) + strl)
    movi = riff_list(b"movi", riff_chunk(b"00wb", samples))
    return riff_chunk(b"RIFF", b"AVI " + hdrl + movi)


def make_y4m_411_and_410_without_f_token() -> tuple[bytes, bytes]:
    # 4x4 C411 with omitted F token (defaults to 25:1): Y=16, U=4, V=4
    header_411 = b"YUV4MPEG2 W4 H4 Ip A1:1 C411\n"
    frame_411 = b"FRAME\n" + bytes(range(16, 32)) + bytes([96, 112, 128, 144]) + bytes([152, 168, 184, 200])
    # 4x4 C410 with omitted F token (defaults to 25:1): Y=16, U=1, V=1
    header_410 = b"YUV4MPEG2 W4 H4 Ip A1:1 C410\n"
    frame_410 = b"FRAME\n" + bytes(range(16, 32)) + bytes([110]) + bytes([170])
    return header_411 + frame_411, header_410 + frame_410


def ebml_id(raw: int) -> bytes:
    out = []
    while raw > 0:
        out.append(raw & 0xFF)
        raw >>= 8
    return bytes(reversed(out))


def ebml_size(length: int) -> bytes:
    if length < 0x7F:
        return bytes([0x80 | length])
    if length < 0x3FFF:
        return bytes([0x40 | (length >> 8), length & 0xFF])
    return bytes([0x20 | (length >> 16), (length >> 8) & 0xFF, length & 0xFF])


def ebml_elem(elem_id: int, payload: bytes) -> bytes:
    return ebml_id(elem_id) + ebml_size(len(payload)) + payload


def ebml_uint(elem_id: int, value: int) -> bytes:
    if value == 0:
        return ebml_elem(elem_id, b"\x00")
    buf = []
    while value > 0:
        buf.append(value & 0xFF)
        value >>= 8
    return ebml_elem(elem_id, bytes(reversed(buf)))


def ebml_str(elem_id: int, text: str) -> bytes:
    return ebml_elem(elem_id, text.encode("utf-8"))


def make_mkv_bitmap_subtitles_and_data_track() -> bytes:
    ebml_header = ebml_elem(
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
        ebml_uint(0x2AD7B1, 1_000_000) + ebml_elem(0x4489, struct.pack(">d", 1000.0)),
    )
    track1 = ebml_elem(
        0xAE,
        ebml_uint(0xD7, 1)
        + ebml_uint(0x73C5, 1)
        + ebml_uint(0x83, 17)  # subtitle
        + ebml_str(0x86, "S_VOBSUB"),
    )
    track2 = ebml_elem(
        0xAE,
        ebml_uint(0xD7, 2)
        + ebml_uint(0x73C5, 2)
        + ebml_uint(0x83, 17)  # subtitle
        + ebml_str(0x86, "S_HDMV/PGS"),
    )
    track3 = ebml_elem(
        0xAE,
        ebml_uint(0xD7, 3)
        + ebml_uint(0x73C5, 3)
        + ebml_uint(0x83, 0x20)  # control / non-AVS track kind
        + ebml_str(0x86, "D_WEBVTT/METADATA"),
    )
    tracks = ebml_elem(0x1654AE6B, track1 + track2 + track3)
    cluster = ebml_elem(
        0x1F43B675,
        ebml_uint(0xE7, 0)
        + ebml_elem(0xA3, bytes([0x81, 0x00, 0x00, 0x80, 0x00, 0x00])),
    )
    segment = ebml_elem(0x18538067, info + tracks + cluster)
    return ebml_header + segment


def main() -> None:
    OUT_DIR.mkdir(parents=True, exist_ok=True)
    (OUT_DIR / "aiff_block_aligned_zero_frames.aiff").write_bytes(
        make_aiff_block_aligned_zero_frames()
    )
    (OUT_DIR / "avi_pcm_4frames.avi").write_bytes(make_avi_pcm_4frames())
    y4m_411, y4m_410 = make_y4m_411_and_410_without_f_token()
    (OUT_DIR / "tiny_411.y4m").write_bytes(y4m_411)
    (OUT_DIR / "tiny_410.y4m").write_bytes(y4m_410)
    (OUT_DIR / "bitmap_subs_and_data_track.mkv").write_bytes(
        make_mkv_bitmap_subtitles_and_data_track()
    )


if __name__ == "__main__":
    main()
