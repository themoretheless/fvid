#!/usr/bin/env python3
"""Generate tiny synthetic fixtures for Batch 589 regressions (FVID-0675..0679).

1. matroska_copy_zlib_video.mkv:
   Minimal Matroska container with a ContentCompAlgo=0 (zlib) video track whose
   compressed Block payload is small (~20 bytes) while decompressed payload is
   4096 bytes.
2. xfade_bad_colorrange.y4m:
   Minimal 2x2 Y4M with an invalid XCOLORRANGE=BOGUS extension tag.
"""

from pathlib import Path
import struct
import zlib


def ebml_vint(value: int) -> bytes:
    for width in range(1, 9):
        limit = (1 << (7 * width)) - 1
        if value < limit:
            marker = 1 << (8 - width)
            out = bytearray(width)
            rem = value
            for idx in range(width - 1, -1, -1):
                out[idx] = rem & 0xFF
                rem >>= 8
            out[0] |= marker
            return bytes(out)
    raise ValueError("vint overflow")


def elem(tag: bytes, payload: bytes) -> bytes:
    return tag + ebml_vint(len(payload)) + payload


def uint_bytes(value: int) -> bytes:
    if value == 0:
        return b"\x00"
    out = bytearray()
    while value:
        out.append(value & 0xFF)
        value >>= 8
    out.reverse()
    return bytes(out)


def build_matroska_copy_zlib_video() -> bytes:
    ebml_head = elem(
        b"\x1a\x45\xdf\xa3",
        elem(b"\x42\x86", b"\x01")
        + elem(b"\x42\xf7", b"\x01")
        + elem(b"\x42\xf2", b"\x04")
        + elem(b"\x42\xf3", b"\x08")
        + elem(b"\x42\x82", b"matroska")
        + elem(b"\x42\x87", b"\x04")
        + elem(b"\x42\x85", b"\x02"),
    )
    info = elem(
        b"\x15\x49\xa9\x66",
        elem(b"\x2a\xd7\xb1", uint_bytes(1_000_000))
        + elem(b"\x4d\x80", b"FVidTest")
        + elem(b"\x57\x41", b"FVidTest"),
    )
    compression = elem(
        b"\x50\x34",
        elem(b"\x42\x54", b"\x00"),  # ContentCompAlgo = 0 (zlib)
    )
    encodings = elem(
        b"\x6d\x80",
        elem(b"\x62\x40", compression),
    )
    video_settings = elem(
        b"\xe0",
        elem(b"\xb0", uint_bytes(16)) + elem(b"\xba", uint_bytes(16)),
    )
    track_entry = elem(
        b"\xae",
        elem(b"\xd7", b"\x01")
        + elem(b"\x73\xc5", b"\x01")
        + elem(b"\x83", b"\x01")
        + elem(b"\x86", b"V_VP8")
        + video_settings
        + encodings,
    )
    tracks = elem(b"\x16\x54\xae\x6b", track_entry)
    raw_payload = b"\x9d\x01\x2a" + (b"\x00" * 4093)
    comp_payload = zlib.compress(raw_payload, level=9)
    simple_block = elem(
        b"\xa3",
        b"\x81\x00\x00\x80" + comp_payload,
    )
    cluster = elem(
        b"\x1f\x43\xb6\x75",
        elem(b"\xe7", b"\x00") + simple_block,
    )
    segment = elem(b"\x18\x53\x80\x67", info + tracks + cluster)
    return ebml_head + segment


def build_xfade_bad_colorrange() -> bytes:
    header = b"YUV4MPEG2 W2 H2 F25:1 Ip A1:1 C420 XCOLORRANGE=BOGUS\n"
    frame = b"FRAME\n" + bytes([128, 128, 128, 128, 128, 128])
    return header + frame + frame


def build_valid_2x2_y4m(fps_num: int = 25) -> bytes:
    header = f"YUV4MPEG2 W2 H2 F{fps_num}:1 Ip A1:1 C420 XCOLORRANGE=LIMITED\n".encode(
        "ascii"
    )
    frame = b"FRAME\n" + bytes([128, 128, 128, 128, 128, 128])
    return header + frame + frame


def build_pcm16_stereo_wav(sample_rate: int = 8000, frames: int = 64) -> bytes:
    channels = 2
    bits = 16
    block_align = channels * (bits // 8)
    byte_rate = sample_rate * block_align
    pcm = b"\x00\x00\x00\x00" * frames
    fmt = struct.pack(
        "<4sIHHIIHH",
        b"fmt ",
        16,
        1,
        channels,
        sample_rate,
        byte_rate,
        block_align,
        bits,
    )
    data = struct.pack("<4sI", b"data", len(pcm)) + pcm
    riff = struct.pack("<4sI4s", b"RIFF", 4 + len(fmt) + len(data), b"WAVE")
    return riff + fmt + data


def main() -> None:
    root = Path(__file__).resolve().parents[1]
    out_dir = root / "tests" / "fixtures" / "playback-errors"
    out_dir.mkdir(parents=True, exist_ok=True)
    (out_dir / "matroska_copy_zlib_video.mkv").write_bytes(
        build_matroska_copy_zlib_video()
    )
    (out_dir / "xfade_bad_colorrange.y4m").write_bytes(
        build_xfade_bad_colorrange()
    )
    (out_dir / "batch589_valid_25fps.y4m").write_bytes(build_valid_2x2_y4m(25))
    (out_dir / "batch589_valid_30fps.y4m").write_bytes(build_valid_2x2_y4m(30))
    (out_dir / "batch589_pcm16_8k.wav").write_bytes(build_pcm16_stereo_wav())



if __name__ == "__main__":
    main()
