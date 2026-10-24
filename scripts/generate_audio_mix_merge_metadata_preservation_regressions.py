#!/usr/bin/env python3
"""Generate synthetic WAVE, MP4 (sowt PCM), and Matroska (A_FLAC) fixtures with first-input metadata for mix-audio and merge-audio (.wav/.mka/.mkv) metadata preservation and selective editing regressions (FVID-0236)."""

from __future__ import annotations

import hashlib
import json
import math
from pathlib import Path
import struct

from generate_audio_resample_window_fixture import ebml
from generate_flac_coded_numbers_regressions import frame as flac_frame, stream as flac_stream


ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / "tests/fixtures/playback-errors/audio-mix-merge-metadata-preservation"


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


def build_wave(
    *,
    bits_per_sample: int,
    freq_hz: float,
    tags: list[tuple[bytes, str]],
) -> bytes:
    assert bits_per_sample in (16, 24)
    sample_rate = 48_000
    channels = 1
    frames = 1920
    byte_width = bits_per_sample // 8
    pcm = bytearray()
    for i in range(frames):
        s = math.sin(2.0 * math.pi * freq_hz * i / sample_rate)
        if bits_per_sample == 16:
            pcm.extend(struct.pack("<h", int(round(s * 9500.0))))
        else:
            val = int(round(s * 2400000.0))
            pcm.extend(val.to_bytes(4, "little", signed=True)[:3])

    fmt_chunk = struct.pack(
        "<HHIIHH",
        1,
        channels,
        sample_rate,
        sample_rate * channels * byte_width,
        channels * byte_width,
        bits_per_sample,
    )
    info_body = b"INFO" + b"".join(info_subchunk(fourcc, text) for fourcc, text in tags)
    list_chunk = b"LIST" + struct.pack("<I", len(info_body)) + info_body
    pad = b"\x00" if len(pcm) % 2 == 1 else b""
    riff_body = (
        b"WAVE"
        + b"fmt "
        + struct.pack("<I", len(fmt_chunk))
        + fmt_chunk
        + list_chunk
        + b"data"
        + struct.pack("<I", len(pcm))
        + bytes(pcm)
        + pad
    )
    return b"RIFF" + struct.pack("<I", len(riff_body)) + riff_body


def pack_mdhd_lang(code: str) -> int:
    assert len(code) == 3
    return (
        (((ord(code[0]) - 0x60) & 31) << 10)
        | (((ord(code[1]) - 0x60) & 31) << 5)
        | ((ord(code[2]) - 0x60) & 31)
    )


def build_mp4_first_input() -> bytes:
    total_samples = 1920
    sowt_payload = b"".join(
        struct.pack("<h", 7200 if (i // 32) % 2 == 0 else -7200) for i in range(total_samples)
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
    trak_udta = box(b"udta", box(b"name", b"MixMerge MP4 Track Title"))
    trak = box(b"trak", box(b"tkhd", bytes(tkhd)) + mdia + trak_udta)
    mvhd = bytearray(100)
    struct.pack_into(">II", mvhd, 12, 48_000, total_samples)
    struct.pack_into(">IH", mvhd, 20, 1 << 16, 256)
    for off, val in [(36, 1 << 16), (52, 1 << 16), (68, 1 << 30), (96, 2)]:
        struct.pack_into(">I", mvhd, off, val)
    moov_udta = box(
        b"udta",
        box(b"\xa9nam", b"MixMerge MP4 Title")
        + box(b"\xa9ART", b"MixMerge MP4 Artist")
        + box(b"\xa9alb", b"MixMerge MP4 Album")
        + box(b"\xa9cmt", b"MixMerge MP4 Comment")
        + box(b"cprt", b"MixMerge MP4 Copyright")
        + box(b"aART", b"MixMerge MP4 Album Artist")
        + box(b"desc", b"MixMerge MP4 Description")
        + box(b"trkn", struct.pack(">HHHH", 0, 5, 9, 0))
        + box(b"disk", struct.pack(">HHH", 0, 1, 2)),
    )
    return mdat + box(b"moov", box(b"mvhd", bytes(mvhd)) + trak + moov_udta)


def simple_tag(name: str, value: str) -> bytes:
    return ebml("67c8", ebml("45a3", name.encode("utf-8")) + ebml("4487", value.encode("utf-8")))


def build_matroska_first_input() -> bytes:
    packet_count = 2
    packets = [
        flac_frame(i, 1920, False, 7200 if i % 2 == 0 else -7200, block_code=7, rate_code=10)
        for i in range(packet_count)
    ]
    raw_flac = flac_stream(packets, [1920] * packet_count, rate=48_000)
    streaminfo = raw_flac[8:42]
    flac_private = b"fLaC\x80\x00\x00\x22" + streaminfo

    header = ebml("1a45dfa3", ebml("4282", b"matroska") + u("4287", 4) + u("4285", 2))
    info = ebml(
        "1549a966",
        u("2ad7b1", 1_000_000) + ebml("5741", b"FVidMixMergeMkaSynth"),
    )
    audio_settings = ebml("e1", ebml("b5", struct.pack(">d", 48000.0)) + u("9f", 1) + u("6264", 16))
    track = ebml(
        "ae",
        u("d7", 1)
        + u("73c5", 501)
        + u("83", 2)
        + ebml("86", b"A_FLAC")
        + ebml("536e", b"MixMerge MKA Track Title")
        + ebml("22b59c", b"deu")
        + ebml("63a2", flac_private)
        + audio_settings,
    )
    tracks = ebml("1654ae6b", track)
    container_tag = ebml(
        "7373",
        ebml("63c0", b"")
        + simple_tag("TITLE", "MixMerge MKA Title")
        + simple_tag("ARTIST", "MixMerge MKA Artist")
        + simple_tag("ALBUM", "MixMerge MKA Album")
        + simple_tag("COMMENT", "MixMerge MKA Comment")
        + simple_tag("COPYRIGHT", "MixMerge MKA Copyright")
        + simple_tag("TRACKNUMBER", "6/12")
        + simple_tag("CUSTOM_MIX_SOURCE", "MixMergeMkaCustomVal")
        + simple_tag("QWER", "MixMergeMkaFourCC"),
    )
    track_tag = ebml(
        "7373",
        ebml("63c0", u("63c5", 501))
        + simple_tag("ISRC", "US-FVD-26-00004")
        + simple_tag("CUSTOM_MIX_TRACK", "MixMergeTrackVal"),
    )
    tags = ebml("1254c367", container_tag + track_tag)
    blocks = u("e7", 0)
    for i, pkt in enumerate(packets):
        blocks += simple_block(1, i * 40, 0x80, pkt)
    return header + ebml("18538067", info + tracks + tags + ebml("1f43b675", blocks))


def generate() -> dict[str, bytes]:
    wave_16 = build_wave(
        bits_per_sample=16,
        freq_hz=500.0,
        tags=[
            (b"INAM", "MixMerge Wave Input 1 Title"),
            (b"IART", "MixMerge Wave Input 1 Artist"),
            (b"IPRD", "MixMerge Wave Input 1 Album"),
            (b"ICMT", "MixMerge Wave Input 1 Comment"),
            (b"ICOP", "MixMerge Wave Input 1 Copyright"),
            (b"ISFT", "FVidMixMergeWaveSynth"),
            (b"ITRK", "2/8"),
            (b"ILNG", "jpn"),
            (b"QWER", "MixMergeWaveFourCC"),
        ],
    )
    wave_24 = build_wave(
        bits_per_sample=24,
        freq_hz=750.0,
        tags=[
            (b"INAM", "MixMerge Wave Input 2 Ignored Title"),
            (b"IART", "MixMerge Wave Input 2 Ignored Artist"),
        ],
    )
    files = {
        "wave-in1-16bit.wav": wave_16,
        "wave-in2-24bit.wav": wave_24,
        "mp4-in1.m4a": build_mp4_first_input(),
        "matroska-in1.mka": build_matroska_first_input(),
    }
    manifest = {
        "batch_id": "FVID-0236",
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
