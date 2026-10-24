#!/usr/bin/env python3
"""Generate synthetic ADTS AAC, G.711 WAVE, Ogg Opus, bare FLAC, Matroska FLAC, and MP4 PCM/ADPCM fixtures for loudnorm budget admission, audio mix/merge to Matroska, and sample-domain audio concat regressions (FVID-0212)."""

from __future__ import annotations

import hashlib
import json
from pathlib import Path
import struct

from generate_audio_reader_eof_regressions import generate as owned_eof
from generate_audio_resample_window_fixture import ebml
from generate_flac_coded_numbers_regressions import frame as flac_frame, stream as flac_stream
from generate_wave_ogg_flac_plan_regressions import make_g711_wave, ogg_page


ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / "tests/fixtures/playback-errors/loudnorm-budget-mix-concat"


def u(tag: str, n: int) -> bytes:
    return ebml(tag, n.to_bytes(max(1, (n.bit_length() + 7) // 8), "big"))


def simple_block(track_no: int, rel_ms: int, flags: int, payload: bytes) -> bytes:
    assert 1 <= track_no <= 126
    return ebml("a3", bytes([0x80 | track_no]) + struct.pack(">hB", rel_ms, flags) + payload)


def box(kind: bytes, data: bytes) -> bytes:
    return struct.pack(">I4s", len(data) + 8, kind) + data


def build_adts_48k() -> bytes:
    """Synthetic 48 kHz mono AAC-LC ADTS stream (3 packets of 1024 samples)."""
    # SCE, tag 0, global gain 100, long sine window, zero scale-factor bands, END.
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
    # sampling_frequency_index = 3 (48000 Hz) -> byte 2 = (1 << 6) | (3 << 2) = 0x4C
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


def build_raw_flac_48k() -> bytes:
    """Bare FLAC stream (16-bit 48 kHz mono) with 2 packets of 1920 samples (3840 samples)."""
    packets = [
        flac_frame(i, 1920, False, 4096 if i % 2 == 0 else -4096, block_code=7, rate_code=10)
        for i in range(2)
    ]
    return flac_stream(packets, [1920] * 2, rate=48_000)


def build_matroska_flac_mka() -> bytes:
    """Single-track Matroska A_FLAC (16-bit 48 kHz mono, 2 packets of 1920 samples)."""
    packets = [
        flac_frame(i, 1920, False, 4096 if i % 2 == 0 else -4096, block_code=7, rate_code=10)
        for i in range(2)
    ]
    raw_flac = flac_stream(packets, [1920] * 2, rate=48_000)
    streaminfo = raw_flac[8:42]
    flac_private = b"fLaC\x80\x00\x00\x22" + streaminfo

    header = ebml("1a45dfa3", ebml("4282", b"matroska") + u("4287", 4) + u("4285", 2))
    info = ebml("1549a966", u("2ad7b1", 1_000_000))
    audio_settings = ebml("e1", ebml("b5", struct.pack(">d", 48000.0)) + u("9f", 1) + u("6264", 16))
    track = ebml(
        "ae",
        u("d7", 1)
        + u("73c5", 201)
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


def build_mp4_pcm_m4a() -> bytes:
    """Single-track MP4 sowt (16-bit 48 kHz mono PCM, 3840 samples)."""
    total_samples = 3840
    sowt_payload = b"".join(
        struct.pack("<h", 4096 if (i // 32) % 2 == 0 else -4096) for i in range(total_samples)
    )
    mdat = box(b"mdat", sowt_payload)
    clock = bytearray(24)
    struct.pack_into(">II", clock, 12, 48_000, total_samples)
    entry = bytearray(28)
    struct.pack_into(">H", entry, 6, 1)
    struct.pack_into(">HH", entry, 16, 1, 16)
    struct.pack_into(">I", entry, 24, 48_000 << 16)
    entry += box(b"enda", struct.pack(">H", 1))
    stsd = box(b"stsd", struct.pack(">II", 0, 1) + box(b"sowt", bytes(entry)))
    stts = box(b"stts", struct.pack(">IIII", 0, 1, 1, total_samples))
    stsc = box(b"stsc", struct.pack(">IIIII", 0, 1, 1, 1, 1))
    stsz = box(b"stsz", struct.pack(">III", 0, len(sowt_payload), 1))
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
    trak = box(b"trak", box(b"tkhd", bytes(tkhd)) + mdia)
    mvhd = bytearray(100)
    struct.pack_into(">II", mvhd, 12, 48_000, total_samples)
    struct.pack_into(">IH", mvhd, 20, 1 << 16, 256)
    for off, val in [(36, 1 << 16), (52, 1 << 16), (68, 1 << 30), (96, 2)]:
        struct.pack_into(">I", mvhd, off, val)
    return mdat + box(b"moov", box(b"mvhd", bytes(mvhd)) + trak)


def generate() -> dict[str, bytes]:
    codes_alaw = bytes((0x80 if (i % 16 < 8) else 0x00) for i in range(1920))
    codes_mulaw = bytes((0x1F if (i % 16 < 8) else 0x9F) for i in range(1920))
    g711_alaw = make_g711_wave(law="alaw", sample_rate=48_000, channels=1, codes=codes_alaw)
    g711_mulaw = make_g711_wave(law="mulaw", sample_rate=48_000, channels=1, codes=codes_mulaw)

    opus_head = b"OpusHead" + bytes([1, 1]) + struct.pack("<HIhB", 0, 48_000, 0, 0)
    opus_tags = b"OpusTags" + struct.pack("<I", 4) + b"fvid" + struct.pack("<I", 0)
    raw20 = bytes.fromhex("f8fffe")
    ogg_opus = b"".join(
        [
            ogg_page(opus_head, 0, 2, 0, 0x46560212),
            ogg_page(opus_tags, 0, 0, 1, 0x46560212),
            ogg_page(raw20, 960, 0, 2, 0x46560212),
            ogg_page(raw20, 1920, 4, 3, 0x46560212),
        ]
    )

    files = {
        "video.mkv": owned_eof()["video.mkv"],
        "adts-mono-48k.aac": build_adts_48k(),
        "g711-alaw-48k.wav": g711_alaw,
        "g711-mulaw-48k.wav": g711_mulaw,
        "ogg-opus-mono-48k.ogg": ogg_opus,
        "raw-flac-48k.flac": build_raw_flac_48k(),
        "matroska-flac-48k.mka": build_matroska_flac_mka(),
        "mp4-pcm-48k.m4a": build_mp4_pcm_m4a(),
    }
    manifest = {
        "batch_id": "FVID-0212",
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
