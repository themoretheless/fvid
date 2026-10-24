#!/usr/bin/env python3
"""Generate synthetic PCM WAVE, ADTS AAC, G.711 WAVE, bare FLAC, Matroska FLAC, and MP4 PCM fixtures for audio budget controls across decode-audio, trim, trim-pcm, loudness, and normalize-loudness (FVID-0214)."""

from __future__ import annotations

import hashlib
import json
from pathlib import Path
import struct

from generate_audio_reader_eof_regressions import generate as owned_eof
from generate_audio_resample_window_fixture import ebml
from generate_flac_coded_numbers_regressions import frame as flac_frame, stream as flac_stream
from generate_media_trim_loudness_regressions import build_raw_flac_48k
from generate_wave_ogg_flac_plan_regressions import make_g711_wave


ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / "tests/fixtures/playback-errors/audio-budget-controls"


def u(tag: str, n: int) -> bytes:
    return ebml(tag, n.to_bytes(max(1, (n.bit_length() + 7) // 8), "big"))


def simple_block(track_no: int, rel_ms: int, flags: int, payload: bytes) -> bytes:
    assert 1 <= track_no <= 126
    return ebml("a3", bytes([0x80 | track_no]) + struct.pack(">hB", rel_ms, flags) + payload)


def box(kind: bytes, data: bytes) -> bytes:
    return struct.pack(">I4s", len(data) + 8, kind) + data


def build_pcm_wave_48k_450ms() -> bytes:
    """Synthetic 16-bit 48 kHz mono PCM WAVE with 21,600 samples (450 ms > 400 ms loudness gate)."""
    total_samples = 21_600
    pcm_bytes = b"".join(
        struct.pack("<h", 4096 if (i // 32) % 2 == 0 else -4096) for i in range(total_samples)
    )
    fmt = struct.pack("<HHIIHH", 1, 1, 48_000, 96_000, 2, 16)
    riff_body = (
        b"WAVE"
        + b"fmt "
        + struct.pack("<I", len(fmt))
        + fmt
        + b"data"
        + struct.pack("<I", len(pcm_bytes))
        + pcm_bytes
    )
    return b"RIFF" + struct.pack("<I", len(riff_body)) + riff_body


def build_adts_48k() -> bytes:
    """Synthetic 48 kHz mono AAC-LC ADTS stream (3 packets of 1024 samples)."""
    fields = [
        (0, 3),
        (0, 4),
        (100, 8),
        (0, 1),
        (0, 2),
        (0, 1),
        (0, 6),
        (0, 1),
        (0, 1),
        (0, 1),
        (0, 1),
        (7, 3),
    ]
    bits = "".join(format(value, f"0{width}b") for value, width in fields)
    bits += "0" * (-len(bits) % 8)
    payload = int(bits, 2).to_bytes(len(bits) // 8, "big")
    size = 7 + len(payload)
    header = bytes(
        [
            0xFF,
            0xF1,
            0x4C,
            0x40 | (size >> 11),
            (size >> 3) & 255,
            ((size & 7) << 5) | 31,
            0xFC,
        ]
    )
    return (header + payload) * 3


def build_mp4_pcm_48k_450ms() -> bytes:
    """Single-track MP4 sowt (16-bit 48 kHz mono PCM, 21,600 samples = 450 ms, 12 packets of 1,800 samples)."""
    total_samples = 21_600
    sowt_payload = b"".join(
        struct.pack("<h", 4096 if (i // 32) % 2 == 0 else -4096) for i in range(total_samples)
    )
    ftyp = box(b"ftyp", b"isom\x00\x00\x02\x00isomiso2mp41")
    mdat = box(b"mdat", sowt_payload)
    clock = bytearray(24)
    struct.pack_into(">II", clock, 12, 48_000, total_samples)
    entry = bytearray(28)
    struct.pack_into(">H", entry, 6, 1)
    struct.pack_into(">HH", entry, 16, 1, 16)
    struct.pack_into(">I", entry, 24, 48_000 << 16)
    entry += box(b"enda", struct.pack(">H", 1))
    stsd = box(b"stsd", struct.pack(">II", 0, 1) + box(b"sowt", bytes(entry)))
    # 12 samples in MP4 table, each 1,800 PCM frames (3,600 bytes)
    stts = box(b"stts", struct.pack(">IIII", 0, 1, 12, 1_800))
    stsc = box(b"stsc", struct.pack(">IIIII", 0, 1, 1, 1, 1))
    stsz = box(b"stsz", struct.pack(">III", 0, 3_600, 12))
    offsets = [len(ftyp) + 8 + i * 3_600 for i in range(12)]
    stco = box(b"stco", struct.pack(">II12I", 0, 12, *offsets))
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
    trak = box(b"trak", box(b"tkhd", bytes(tkhd)) + mdia)
    mvhd = bytearray(100)
    struct.pack_into(">II", mvhd, 12, 48_000, total_samples)
    struct.pack_into(">IH", mvhd, 20, 1 << 16, 256)
    for off, val in [(36, 1 << 16), (52, 1 << 16), (68, 1 << 30), (96, 2)]:
        struct.pack_into(">I", mvhd, off, val)
    return ftyp + mdat + box(b"moov", box(b"mvhd", bytes(mvhd)) + trak)


def build_matroska_flac_48k_440ms() -> bytes:
    """Single-track Matroska A_FLAC (16-bit 48 kHz mono, 11 packets of 1,920 samples = 440 ms)."""
    packets = [
        flac_frame(i, 1920, False, 4096 if i % 2 == 0 else -4096, block_code=7, rate_code=10)
        for i in range(11)
    ]
    raw_flac = flac_stream(packets, [1920] * 11, rate=48_000)
    streaminfo = raw_flac[8:42]
    flac_private = b"fLaC\x80\x00\x00\x22" + streaminfo

    header = ebml("1a45dfa3", ebml("4282", b"matroska") + u("4287", 4) + u("4285", 2))
    info = ebml("1549a966", u("2ad7b1", 1_000_000))
    audio_settings = ebml("e1", ebml("b5", struct.pack(">d", 48000.0)) + u("9f", 1) + u("6264", 16))
    track = ebml(
        "ae",
        u("d7", 1)
        + u("73c5", 301)
        + u("83", 2)
        + ebml("86", b"A_FLAC")
        + ebml("63a2", flac_private)
        + audio_settings,
    )
    tracks = ebml("1654ae6b", track)
    blocks = u("e7", 0)
    for i, pkt in enumerate(packets):
        blocks += simple_block(1, i * 40, 0x80, pkt)
    return header + ebml("18538067", info + tracks + ebml("1f43b675", blocks))


def generate() -> dict[str, bytes]:
    codes_alaw = bytes((0x80 if (i % 16 < 8) else 0x00) for i in range(43_200))
    g711_alaw = make_g711_wave(law="alaw", sample_rate=48_000, channels=2, codes=codes_alaw)

    return {
        "video.mkv": owned_eof()["video.mkv"],
        "pcm-s16le-48k-450ms.wav": build_pcm_wave_48k_450ms(),
        "aac-lc-48k.adts": build_adts_48k(),
        "g711-alaw-48k-450ms.wav": g711_alaw,
        "raw-flac-48k-440ms.flac": build_raw_flac_48k(),
        "matroska-flac-48k-440ms.mka": build_matroska_flac_48k_440ms(),
        "mp4-pcm-48k-450ms.m4a": build_mp4_pcm_48k_450ms(),
    }


def write_fixtures(out_dir: Path = OUT) -> dict[str, str]:
    out_dir.mkdir(parents=True, exist_ok=True)
    files = generate()
    hashes: dict[str, str] = {}
    for name, content in sorted(files.items()):
        target = out_dir / name
        target.write_bytes(content)
        hashes[name] = hashlib.sha256(content).hexdigest()
    manifest = {
        "batch_id": "FVID-0214",
        "sha256": hashes,
    }
    (out_dir / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8")
    return hashes


if __name__ == "__main__":
    write_fixtures()
