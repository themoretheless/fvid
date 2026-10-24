#!/usr/bin/env python3
"""Generate synthetic WAVE, MP4 (sowt PCM), and Matroska (A_FLAC) fixtures with rich container and stream metadata for Matroska PCM (.mka/.mkv) metadata preservation and selective editing regressions (FVID-0234)."""

from __future__ import annotations

import hashlib
import json
import math
from pathlib import Path
import struct

from generate_audio_resample_window_fixture import ebml
from generate_flac_coded_numbers_regressions import frame as flac_frame, stream as flac_stream


ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / "tests/fixtures/playback-errors/audio-matroska-pcm-metadata-preservation"


def u(tag: str, n: int) -> bytes:
    return ebml(tag, n.to_bytes(max(1, (n.bit_length() + 7) // 8), "big"))


def simple_block(track_no: int, rel_ms: int, flags: int, payload: bytes) -> bytes:
    assert 1 <= track_no <= 126
    return ebml("a3", bytes([0x80 | track_no]) + struct.pack(">hB", rel_ms, flags) + payload)


def box(kind: bytes, data: bytes) -> bytes:
    return struct.pack(">I4s", len(data) + 8, kind) + data


def info_subchunk(fourcc: bytes, text: str) -> bytes:
    assert len(fourcc) == 4
    raw = text.encode("utf-8") + b"\x00"
    padded = raw + (b"\x00" if len(raw) % 2 == 1 else b"")
    return fourcc + struct.pack("<I", len(raw)) + padded


def build_wave_mka_preserve() -> bytes:
    sample_rate = 48_000
    channels = 1
    bits_per_sample = 16
    frames = 24_000  # 0.5 s at 48 kHz (exceeds 400 ms BS.1770 gating block)
    pcm = bytearray()
    for i in range(frames):
        val = int(round(8192.0 * math.sin(2.0 * math.pi * 440.0 * i / sample_rate)))
        pcm.extend(struct.pack("<h", val))

    fmt_chunk = struct.pack(
        "<HHIIHH",
        1,
        channels,
        sample_rate,
        sample_rate * channels * (bits_per_sample // 8),
        channels * (bits_per_sample // 8),
        bits_per_sample,
    )
    info_body = (
        b"INFO"
        + info_subchunk(b"INAM", "Wave To MKA Title")
        + info_subchunk(b"IART", "Wave To MKA Artist")
        + info_subchunk(b"IPRD", "Wave To MKA Album")
        + info_subchunk(b"ICMT", "Wave To MKA Comment")
        + info_subchunk(b"ICOP", "Wave To MKA Copyright")
        + info_subchunk(b"ISFT", "FVidWaveSynth")
        + info_subchunk(b"ITRK", "2/8")
        + info_subchunk(b"ILNG", "eng")
        + info_subchunk(b"QWER", "WaveFourCCValue")
    )
    list_chunk = b"LIST" + struct.pack("<I", len(info_body)) + info_body
    data_chunk = b"data" + struct.pack("<I", len(pcm)) + bytes(pcm)
    riff_body = (
        b"WAVE"
        + b"fmt "
        + struct.pack("<I", len(fmt_chunk))
        + fmt_chunk
        + list_chunk
        + data_chunk
    )
    return b"RIFF" + struct.pack("<I", len(riff_body)) + riff_body


def pack_mdhd_lang(code: str) -> int:
    assert len(code) == 3
    return (
        (((ord(code[0]) - 0x60) & 31) << 10)
        | (((ord(code[1]) - 0x60) & 31) << 5)
        | ((ord(code[2]) - 0x60) & 31)
    )


def build_mp4_mka_preserve() -> bytes:
    """Single-track MP4 sowt (16-bit 48 kHz mono PCM, 24000 samples = 0.5 s) with full moov/udta tags (including album_artist, disc, publisher, description) and trak/udta/name + mdhd language."""
    total_samples = 24_000
    sowt_payload = b"".join(
        struct.pack("<h", 8192 if (i // 32) % 2 == 0 else -8192) for i in range(total_samples)
    )
    mdat = box(b"mdat", sowt_payload)
    clock = bytearray(24)
    struct.pack_into(">II", clock, 12, 48_000, total_samples)
    struct.pack_into(">H", clock, 20, pack_mdhd_lang("fra"))
    entry = bytearray(28)
    struct.pack_into(">H", entry, 6, 1)
    struct.pack_into(">HH", entry, 16, 1, 16)
    struct.pack_into(">I", entry, 24, 48_000 << 16)
    entry += box(b"enda", struct.pack(">H", 1))
    stsd = box(b"stsd", struct.pack(">II", 0, 1) + box(b"sowt", bytes(entry)))
    stts = box(b"stts", struct.pack(">IIII", 0, 1, total_samples, 1))
    stsc = box(b"stsc", struct.pack(">IIIII", 0, 1, 1, total_samples, 1))
    stsz = box(b"stsz", struct.pack(">III", 0, 2, total_samples))
    stco = box(b"stco", struct.pack(">III", 0, 1, 8))
    stbl = box(b"stbl", stsd + stts + stsc + stsz + stco)
    hdlr = box(b"hdlr", bytes(8) + b"soun" + bytes(12))
    dinf = box(b"dinf", box(b"dref", struct.pack(">II", 0, 1) + box(b"url ", struct.pack(">I", 1))))
    mdia = box(
        b"mdia",
        box(b"mdhd", bytes(clock)) + hdlr + box(b"minf", box(b"smhd", bytes(8)) + dinf + stbl),
    )
    tkhd = bytearray(84)
    tkhd[3] = 7
    struct.pack_into(">H", tkhd, 36, 256)
    struct.pack_into(">I", tkhd, 12, 1)
    struct.pack_into(">I", tkhd, 20, total_samples)
    for off, val in [(40, 1 << 16), (56, 1 << 16), (72, 1 << 30)]:
        struct.pack_into(">I", tkhd, off, val)
    trak_udta = box(b"udta", box(b"name", b"MP4 Distinct Track Name"))
    trak = box(b"trak", box(b"tkhd", bytes(tkhd)) + mdia + trak_udta)
    mvhd = bytearray(100)
    struct.pack_into(">II", mvhd, 12, 48_000, total_samples)
    struct.pack_into(">IH", mvhd, 20, 1 << 16, 256)
    for off, val in [(36, 1 << 16), (52, 1 << 16), (68, 1 << 30), (96, 2)]:
        struct.pack_into(">I", mvhd, off, val)
    moov_udta = box(
        b"udta",
        box(b"\xa9nam", b"MP4 To MKA Title")
        + box(b"\xa9ART", b"MP4 To MKA Artist")
        + box(b"\xa9alb", b"MP4 To MKA Album")
        + box(b"\xa9day", b"2026")
        + box(b"\xa9cmt", b"MP4 To MKA Comment")
        + box(b"cprt", b"MP4 To MKA Copyright")
        + box(b"aART", b"MP4 To MKA Album Artist")
        + box(b"\xa9pub", b"MP4 To MKA Publisher")
        + box(b"desc", b"MP4 To MKA Description")
        + box(b"trkn", struct.pack(">HHHH", 0, 5, 12, 0))
        + box(b"disk", struct.pack(">HHH", 0, 1, 2)),
    )
    return mdat + box(b"moov", box(b"mvhd", bytes(mvhd)) + trak + moov_udta)


def simple_tag(name: str, value: str) -> bytes:
    return ebml("67c8", ebml("45a3", name.encode("utf-8")) + ebml("4487", value.encode("utf-8")))


def build_matroska_mka_preserve() -> bytes:
    """Single-track Matroska A_FLAC (16-bit 48 kHz mono, 13 packets of 1920 samples = 24960 samples > 0.5 s) with Segment and Track tags."""
    packet_count = 13
    packets = [
        flac_frame(i, 1920, False, 8192 if i % 2 == 0 else -8192, block_code=7, rate_code=10)
        for i in range(packet_count)
    ]
    raw_flac = flac_stream(packets, [1920] * packet_count, rate=48_000)
    streaminfo = raw_flac[8:42]
    flac_private = b"fLaC\x80\x00\x00\x22" + streaminfo

    header = ebml("1a45dfa3", ebml("4282", b"matroska") + u("4287", 4) + u("4285", 2))
    info = ebml(
        "1549a966",
        u("2ad7b1", 1_000_000) + ebml("5741", b"FVidMkaSynthMuxer"),
    )
    audio_settings = ebml("e1", ebml("b5", struct.pack(">d", 48000.0)) + u("9f", 1) + u("6264", 16))
    track = ebml(
        "ae",
        u("d7", 1)
        + u("73c5", 301)
        + u("83", 2)
        + ebml("86", b"A_FLAC")
        + ebml("536e", b"MKA Distinct Track Name")
        + ebml("22b59c", b"deu")
        + ebml("63a2", flac_private)
        + audio_settings,
    )
    tracks = ebml("1654ae6b", track)
    container_tag = ebml(
        "7373",
        ebml("63c0", b"")
        + simple_tag("TITLE", "MKA To MKA Title")
        + simple_tag("ARTIST", "MKA To MKA Artist")
        + simple_tag("ALBUM", "MKA To MKA Album")
        + simple_tag("ALBUM_ARTIST", "MKA To MKA Album Artist")
        + simple_tag("DISCNUMBER", "2/3")
        + simple_tag("PUBLISHER", "MKA To MKA Publisher")
        + simple_tag("DESCRIPTION", "MKA To MKA Description")
        + simple_tag("RATING", "5.0")
        + simple_tag("COMMENT", "MKA To MKA Comment")
        + simple_tag("DATE", "2025")
        + simple_tag("GENRE", "Synth")
        + simple_tag("COPYRIGHT", "MKA To MKA Copyright")
        + simple_tag("TRACKNUMBER", "3/9")
        + simple_tag("CUSTOM_CONTAINER_NOTE", "ContainerNoteValue"),
    )
    track_tag = ebml(
        "7373",
        ebml("63c0", u("63c5", 301))
        + simple_tag("ISRC", "US-FVD-26-00002")
        + simple_tag("CUSTOM_TRACK_NOTE", "TrackNoteValue"),
    )
    tags = ebml("1254c367", container_tag + track_tag)
    blocks = u("e7", 0)
    for i, pkt in enumerate(packets):
        blocks += simple_block(1, i * 40, 0x80, pkt)
    return header + ebml("18538067", info + tracks + tags + ebml("1f43b675", blocks))


def generate() -> dict[str, bytes]:
    files = {
        "wave-mka-preserve.wav": build_wave_mka_preserve(),
        "mp4-mka-preserve.m4a": build_mp4_mka_preserve(),
        "matroska-mka-preserve.mka": build_matroska_mka_preserve(),
    }
    manifest = {
        "batch_id": "FVID-0234",
        "sha256": {name: hashlib.sha256(data).hexdigest() for name, data in files.items()},
    }
    files["manifest.json"] = (json.dumps(manifest, indent=2, sort_keys=True) + "\n").encode("utf-8")
    return files


def main() -> None:
    OUT.mkdir(parents=True, exist_ok=True)
    for name, data in generate().items():
        (OUT / name).write_bytes(data)


if __name__ == "__main__":
    main()
