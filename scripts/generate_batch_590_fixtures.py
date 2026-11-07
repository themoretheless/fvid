#!/usr/bin/env python3
"""Generate tiny synthetic fixtures for Batch 590 regressions (FVID-0680..0684).

1. batch590_multitrack_240_audio_tags.mkv:
   Minimal 2-track Matroska file (16x16 V_FFV1 video + A_OPUS audio) where
   audio TrackUID=2 has 240 scoped SimpleTags in the source container.
2. batch590_mp4_uniform_relocate.mp4:
   Minimal MP4 container with `mdat` BEFORE `moov` (requiring fast-start
   relocation) and a uniform `stsz` sample table (`sample_size=256, count=4`).
"""

from pathlib import Path
import struct


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


def mp4_box(fourcc: bytes, payload: bytes) -> bytes:
    return struct.pack(">I4s", len(payload) + 8, fourcc) + payload


def mp4_fullbox(fourcc: bytes, version: int, flags: int, payload: bytes) -> bytes:
    header = struct.pack(">BBBB", version, (flags >> 16) & 0xFF, (flags >> 8) & 0xFF, flags & 0xFF)
    return mp4_box(fourcc, header + payload)


def build_mp4_uniform_relocate() -> bytes:
    ftyp = mp4_box(b"ftyp", b"isom\x00\x00\x02\x00isomiso2")
    mdat_payload = bytes([i & 0xFF for i in range(4 * 256)])
    mdat_offset = len(ftyp) + 8
    mdat = mp4_box(b"mdat", mdat_payload)

    mvhd = mp4_fullbox(
        b"mvhd",
        0,
        0,
        struct.pack(
            ">IIIIIH10s9I7I",
            0,
            0,
            48000,
            4096,
            0x00010000,
            0x0100,
            b"\x00" * 10,
            0x00010000,
            0,
            0,
            0,
            0x00010000,
            0,
            0,
            0,
            0x40000000,
            0,
            0,
            0,
            0,
            0,
            0,
            2,
        ),
    )
    tkhd = mp4_fullbox(
        b"tkhd",
        0,
        3,
        struct.pack(
            ">IIIII8sHHHH36sII",
            0,
            0,
            1,
            0,
            4096,
            b"\x00" * 8,
            0,
            0,
            0x0100,
            0,
            struct.pack(
                ">9I",
                0x00010000,
                0,
                0,
                0,
                0x00010000,
                0,
                0,
                0,
                0x40000000,
            ),
            0,
            0,
        ),
    )
    mdhd = mp4_fullbox(
        b"mdhd",
        0,
        0,
        struct.pack(">IIIIHH", 0, 0, 48000, 4096, 0x55C4, 0),
    )
    hdlr = mp4_fullbox(
        b"hdlr",
        0,
        0,
        struct.pack(">I4s12s", 0, b"soun", b"\x00" * 12) + b"SoundHandler\x00",
    )
    smhd = mp4_fullbox(b"smhd", 0, 0, struct.pack(">HH", 0, 0))
    dref = mp4_fullbox(
        b"dref",
        0,
        0,
        struct.pack(">I", 1) + mp4_fullbox(b"url ", 0, 1, b""),
    )
    dinf = mp4_box(b"dinf", dref)

    # Minimal mp4a sample entry with 2-byte AudioSpecificConfig 0x1190 (AAC-LC 48kHz stereo)
    esds_payload = bytes(
        [
            0x03,
            0x19,
            0x00,
            0x01,
            0x00,
            0x04,
            0x11,
            0x40,
            0x15,
            0x00,
            0x00,
            0x00,
            0x00,
            0x01,
            0xF4,
            0x00,
            0x00,
            0x01,
            0xF4,
            0x00,
            0x05,
            0x02,
            0x11,
            0x90,
            0x06,
            0x01,
            0x02,
        ]
    )
    esds = mp4_fullbox(b"esds", 0, 0, esds_payload)
    mp4a = mp4_box(
        b"mp4a",
        b"\x00" * 6
        + struct.pack(">HIIHHHHI", 1, 0, 0, 2, 16, 0, 0, 48000 << 16)
        + esds,
    )
    stsd = mp4_fullbox(b"stsd", 0, 0, struct.pack(">I", 1) + mp4a)
    stts = mp4_fullbox(b"stts", 0, 0, struct.pack(">III", 1, 4, 1024))
    stsc = mp4_fullbox(b"stsc", 0, 0, struct.pack(">IIII", 1, 1, 4, 1))
    # Uniform stsz: sample_size = 256, sample_count = 4
    stsz = mp4_fullbox(b"stsz", 0, 0, struct.pack(">II", 256, 4))
    stco = mp4_fullbox(b"stco", 0, 0, struct.pack(">II", 1, mdat_offset))
    stbl = mp4_box(b"stbl", stsd + stts + stsc + stsz + stco)
    minf = mp4_box(b"minf", smhd + dinf + stbl)
    mdia = mp4_box(b"mdia", mdhd + hdlr + minf)
    trak = mp4_box(b"trak", tkhd + mdia)
    moov = mp4_box(b"moov", mvhd + trak)
    return ftyp + mdat + moov


def main() -> None:
    out_dir = (
        Path(__file__).resolve().parent.parent
        / "tests"
        / "fixtures"
        / "playback-errors"
    )
    out_dir.mkdir(parents=True, exist_ok=True)
    (out_dir / "batch590_mp4_uniform_relocate.mp4").write_bytes(
        build_mp4_uniform_relocate()
    )


if __name__ == "__main__":
    main()
