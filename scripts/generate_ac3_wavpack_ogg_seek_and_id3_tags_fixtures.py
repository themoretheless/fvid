#!/usr/bin/env python3
"""Generate synthetic fixtures for AC-3, WavPack, and Ogg Opus sub-sample seek_time and MP3 ID3v2.4 footer/TCON metadata."""

from __future__ import annotations

import pathlib
import struct


ROOT = pathlib.Path(__file__).resolve().parents[1]
OUT_DIR = ROOT / "tests" / "fixtures" / "playback-errors" / "ac3-wavpack-ogg-seek-and-id3-tags"


def make_ac3_1frame() -> bytes:
    # AC-3 syncword = 0x0B77, crc1 = 0x0000
    # fscod = 0 (48000 Hz), frmsizecod = 0 (64 words = 128 bytes) -> byte 4 = 0x00
    # bsid = 8, bsmod = 0 -> byte 5 = 0x40
    # acmod = 1 (mono), lfeon = 0 -> byte 6 = 0x20
    header = bytes([0x0B, 0x77, 0x00, 0x00, 0x00, 0x40, 0x20])
    return header + (b"\x00" * (128 - len(header)))


def wv_item(item_id: int, body: bytes) -> bytes:
    odd = len(body) & 1
    padded = body + (b"\x00" if odd else b"")
    word_size = len(padded) // 2
    tag = item_id | (0x40 if odd else 0)
    return bytes([tag, word_size]) + padded


def make_wavpack_1block() -> bytes:
    items = (
        wv_item(0x02, b"\x01")
        + wv_item(0x03, b"\x00")
        + wv_item(0x04, b"\x00\x00")
        + wv_item(0x05, b"\x00" * 6)
        + wv_item(0x0A, b"\x00\x00")
    )
    ck_size = 24 + len(items)
    version = 0x0410
    total_samples = 4
    block_index = 0
    block_samples = 4
    # 16-bit (1), MONO (1<<2), INITIAL_BLOCK (1<<11), FINAL_BLOCK (1<<12), 48000 Hz (10<<23)
    flags = 1 | (1 << 2) | (1 << 11) | (1 << 12) | (10 << 23)
    crc = 0
    header = struct.pack(
        "<4sIHBBI",
        b"wvpk",
        ck_size,
        version,
        0,
        0,
        total_samples,
    ) + struct.pack("<IIII", block_index, block_samples, flags, crc)
    return header + items


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


def make_ogg_opus_1packet() -> bytes:
    opus_head = b"OpusHead" + struct.pack("<BBHIhB", 1, 1, 0, 48000, 0, 0)
    opus_tags = b"OpusTags" + struct.pack("<I", 4) + b"fvid" + struct.pack("<I", 0)
    # TOC = 0xFC (48 kHz SILK/CELT 20 ms = 960 samples, 1 frame)
    audio_pkt = bytes([0xFC, 0x00])
    p0 = ogg_page(1, 0, 0, 0x02, [opus_head])
    p1 = ogg_page(1, 1, 0, 0x00, [opus_tags])
    p2 = ogg_page(1, 2, 960, 0x04, [audio_pkt])
    return p0 + p1 + p2


def synchsafe(n: int) -> bytes:
    return bytes([
        (n >> 21) & 0x7F,
        (n >> 14) & 0x7F,
        (n >> 7) & 0x7F,
        n & 0x7F,
    ])


def id3v24_frame(frame_id: bytes, text: str) -> bytes:
    payload = b"\x03" + text.encode("utf-8")
    return frame_id + synchsafe(len(payload)) + b"\x00\x00" + payload


def make_mp3_with_id3v24_footer_and_numeric_tcon() -> bytes:
    frames = id3v24_frame(b"TIT2", "Footer Song") + id3v24_frame(b"TCON", "(17)")
    # flags = 0x10 (footer present)
    head_tail = bytes([4, 0, 0x10]) + synchsafe(len(frames))
    id3_tag = b"ID3" + head_tail + frames + b"3DI" + head_tail
    # 1 MPEG-1 Layer II frame (32 kbps, 48000 Hz, mono -> 96 bytes)
    mp2_frame = bytes([0xFF, 0xFD, 0x14, 0xC0]) + (b"\x00" * 92)
    return id3_tag + mp2_frame


def main() -> None:
    OUT_DIR.mkdir(parents=True, exist_ok=True)
    (OUT_DIR / "tiny_48k.ac3").write_bytes(make_ac3_1frame())
    (OUT_DIR / "tiny_48k.wv").write_bytes(make_wavpack_1block())
    (OUT_DIR / "tiny_48k.opus").write_bytes(make_ogg_opus_1packet())
    (OUT_DIR / "id3v24_footer_and_tcon.mp2").write_bytes(
        make_mp3_with_id3v24_footer_and_numeric_tcon()
    )


if __name__ == "__main__":
    main()
