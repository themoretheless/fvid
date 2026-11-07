#!/usr/bin/env python3
"""Generate synthetic fixtures for MP4 G.711 probe, ID3v1/ID3v2 extended genre codes, Vorbis/ID3 metadata aliases, and SMF/XM sub-sample seek_time."""

from __future__ import annotations

import pathlib
import struct


ROOT = pathlib.Path(__file__).resolve().parents[1]
OUT_DIR = ROOT / "tests" / "fixtures" / "playback-errors" / "mp4-g711-probe-id3-vorbis-and-smf-xm-seek"


def atom(fourcc: bytes, payload: bytes) -> bytes:
    return struct.pack(">I", 8 + len(payload)) + fourcc + payload


def full_atom(fourcc: bytes, version: int, flags: int, payload: bytes) -> bytes:
    head = bytes([version]) + flags.to_bytes(3, "big")
    return atom(fourcc, head + payload)


def make_mp4_g711(codec_fourcc: bytes) -> bytes:
    ftyp = atom(b"ftyp", b"qt  " + struct.pack(">I", 0x00000200) + b"qt  ")
    samples = b"\xd5\x55\xd5\x55"
    mdat = atom(b"mdat", samples)
    mdat_data_offset = len(ftyp) + 8

    mvhd = full_atom(
        b"mvhd",
        0,
        0,
        struct.pack(">IIIIIH", 0, 0, 8000, 4, 0x00010000, 0x0100)
        + b"\x00" * 10
        + struct.pack(">9I", 0x00010000, 0, 0, 0, 0x00010000, 0, 0, 0, 0x40000000)
        + b"\x00" * 24
        + struct.pack(">I", 2),
    )

    tkhd = full_atom(
        b"tkhd",
        0,
        3,
        struct.pack(">IIIII", 0, 0, 1, 0, 4)
        + b"\x00" * 8
        + struct.pack(">HHHH", 0, 0, 0x0100, 0)
        + struct.pack(">9I", 0x00010000, 0, 0, 0, 0x00010000, 0, 0, 0, 0x40000000)
        + struct.pack(">II", 0, 0),
    )

    mdhd = full_atom(
        b"mdhd",
        0,
        0,
        struct.pack(">IIIIHH", 0, 0, 8000, 4, 0x55C4, 0),
    )
    hdlr = full_atom(
        b"hdlr",
        0,
        0,
        b"\x00\x00\x00\x00soun" + b"\x00" * 12 + b"SoundHandler\x00",
    )
    smhd = full_atom(b"smhd", 0, 0, struct.pack(">HH", 0, 0))
    url_box = full_atom(b"url ", 0, 1, b"")
    dref = full_atom(b"dref", 0, 0, struct.pack(">I", 1) + url_box)
    dinf = atom(b"dinf", dref)

    # AudioSampleEntry v0: 6 reserved + 2 dref_idx + 8 reserved + 2 channels(1) + 2 sample_size(8) + 2 pre_defined + 2 reserved + 4 sample_rate(8000<<16)
    audio_entry_body = (
        b"\x00" * 6
        + struct.pack(">H", 1)
        + b"\x00" * 8
        + struct.pack(">HHHHI", 1, 8, 0, 0, 8000 << 16)
    )
    sample_entry = atom(codec_fourcc, audio_entry_body)
    stsd = full_atom(b"stsd", 0, 0, struct.pack(">I", 1) + sample_entry)
    stts = full_atom(b"stts", 0, 0, struct.pack(">III", 1, 4, 1))
    stsc = full_atom(b"stsc", 0, 0, struct.pack(">IIII", 1, 1, 4, 1))
    stsz = full_atom(b"stsz", 0, 0, struct.pack(">II", 1, 4))
    stco = full_atom(b"stco", 0, 0, struct.pack(">II", 1, mdat_data_offset))
    stbl = atom(b"stbl", stsd + stts + stsc + stsz + stco)
    minf = atom(b"minf", smhd + dinf + stbl)
    mdia = atom(b"mdia", mdhd + hdlr + minf)
    trak = atom(b"trak", tkhd + mdia)
    moov = atom(b"moov", mvhd + trak)
    return ftyp + mdat + moov


def synchsafe(n: int) -> bytes:
    return bytes([
        (n >> 21) & 0x7F,
        (n >> 14) & 0x7F,
        (n >> 7) & 0x7F,
        n & 0x7F,
    ])


def id3v24_frame(frame_id: bytes, payload: bytes) -> bytes:
    return frame_id + synchsafe(len(payload)) + b"\x00\x00" + payload


def id3v24_txxx(desc: str, val: str) -> bytes:
    payload = b"\x03" + desc.encode("utf-8") + b"\x00" + val.encode("utf-8")
    return id3v24_frame(b"TXXX", payload)


def make_mp2_with_id3_extended_genre_and_txxx_aliases() -> bytes:
    tcon = id3v24_frame(b"TCON", b"\x03(43)")  # 43 = Punk
    txxx = (
        id3v24_txxx("YEAR", "1999")
        + id3v24_txxx("ORGANIZATION", "Indie Label")
        + id3v24_txxx("ENCODED-BY", "fvid-id3-test")
    )
    frames = tcon + txxx
    id3v2 = b"ID3" + bytes([4, 0, 0]) + synchsafe(len(frames)) + frames
    mp2_frame = bytes([0xFF, 0xFD, 0x14, 0xC0]) + (b"\x00" * 92)
    return id3v2 + mp2_frame


def ogg_crc(data: bytes) -> int:
    crc = 0
    for byte in data:
        crc = ((crc << 8) & 0xFFFFFFFF) ^ _OGG_CRC_TABLE[((crc >> 24) ^ byte) & 0xFF]
    return crc


def _build_ogg_crc_table() -> list[int]:
    table = []
    for idx in range(256):
        word = idx << 24
        for _ in range(8):
            if word & 0x80000000:
                word = ((word << 1) ^ 0x04C11DB7) & 0xFFFFFFFF
            else:
                word = (word << 1) & 0xFFFFFFFF
        table.append(word)
    return table


_OGG_CRC_TABLE = _build_ogg_crc_table()


def ogg_page(serial: int, seq: int, granule: int, flags: int, packets: list[bytes]) -> bytes:
    segments = bytearray()
    body = bytearray()
    for pkt in packets:
        body.extend(pkt)
        rem = len(pkt)
        while rem >= 255:
            segments.append(255)
            rem -= 255
        segments.append(rem)
    header_without_crc = (
        b"OggS"
        + bytes([0, flags])
        + struct.pack("<qII", granule, serial, seq)
    )
    tail = bytes([len(segments)]) + bytes(segments) + bytes(body)
    crc = ogg_crc(header_without_crc + b"\x00\x00\x00\x00" + tail)
    return header_without_crc + struct.pack("<I", crc) + tail


def make_ogg_opus_with_vorbis_aliases() -> bytes:
    opus_head = b"OpusHead" + struct.pack("<BBHIhB", 1, 1, 0, 48000, 0, 0)
    comments = [
        b"YEAR=2001",
        b"ORGANIZATION=TestOrg",
        b"ENCODED_BY=fvid-opus-test",
        b"PART_NUMBER=7",
    ]
    tags_body = bytearray()
    tags_body.extend(b"OpusTags")
    tags_body.extend(struct.pack("<I", 4) + b"fvid")
    tags_body.extend(struct.pack("<I", len(comments)))
    for c in comments:
        tags_body.extend(struct.pack("<I", len(c)) + c)
    audio_pkt = bytes([0xFC, 0x00])
    p0 = ogg_page(1, 0, 0, 0x02, [opus_head])
    p1 = ogg_page(1, 1, 0, 0x00, [bytes(tags_body)])
    p2 = ogg_page(1, 2, 960, 0x04, [audio_pkt])
    return p0 + p1 + p2


def vlq(v: int) -> bytes:
    parts = [v & 0x7F]
    v >>= 7
    while v > 0:
        parts.append((v & 0x7F) | 0x80)
        v >>= 7
    return bytes(reversed(parts))


def make_tiny_mid() -> bytes:
    # 1 track, 480 ticks/quarter, 120 bpm (500,000 us/quarter) -> 960 ticks = 1.0 s (1000 ms = 10 windows + 1 release window)
    mthd = b"MThd" + struct.pack(">IHHH", 6, 0, 1, 480)
    events = (
        vlq(0) + b"\xFF\x51\x03" + (500_000).to_bytes(3, "big")
        + vlq(0) + b"\x90\x45\x64"
        + vlq(960) + b"\x80\x45\x40"
        + vlq(0) + b"\xFF\x2F\x00"
    )
    mtrk = b"MTrk" + struct.pack(">I", len(events)) + events
    return mthd + mtrk


def make_tiny_xm() -> bytes:
    # Minimal valid XM v1.04 with 1 order, 1 pattern of 4 rows, 0 instruments, 2 channels, speed=6, bpm=125 (120 ms per row)
    header = bytearray(60 + 276)
    header[0:17] = b"Extended Module: "
    header[17:24] = b"tiny-xm"
    header[37] = 0x1A
    header[38:42] = b"fvid"
    struct.pack_into("<H", header, 58, 0x0104)
    struct.pack_into("<I", header, 60, 276)
    struct.pack_into("<H", header, 64, 1)    # song_length = 1
    struct.pack_into("<H", header, 66, 0)    # restart = 0
    struct.pack_into("<H", header, 68, 2)    # channels = 2
    struct.pack_into("<H", header, 70, 1)    # patterns = 1
    struct.pack_into("<H", header, 72, 0)    # instruments = 0
    struct.pack_into("<H", header, 74, 1)    # flags = 1 (linear)
    struct.pack_into("<H", header, 76, 6)    # speed = 6
    struct.pack_into("<H", header, 78, 125)  # bpm = 125
    header[80] = 0                           # order[0] = pattern 0

    # Pattern 0 header: header_len=9, packing=0, rows=4, packed_size=0
    pat0 = struct.pack("<IBHH", 9, 0, 4, 0)
    return bytes(header) + pat0


def main() -> None:
    OUT_DIR.mkdir(parents=True, exist_ok=True)
    (OUT_DIR / "tiny_alaw.mov").write_bytes(make_mp4_g711(b"alaw"))
    (OUT_DIR / "tiny_ulaw.mov").write_bytes(make_mp4_g711(b"ulaw"))
    (OUT_DIR / "id3_punk_and_aliases.mp2").write_bytes(
        make_mp2_with_id3_extended_genre_and_txxx_aliases()
    )
    (OUT_DIR / "vorbis_aliases.opus").write_bytes(make_ogg_opus_with_vorbis_aliases())
    (OUT_DIR / "tiny.mid").write_bytes(make_tiny_mid())
    (OUT_DIR / "tiny.xm").write_bytes(make_tiny_xm())


if __name__ == "__main__":
    main()
