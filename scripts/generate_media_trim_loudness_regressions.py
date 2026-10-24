#!/usr/bin/env python3
"""Generate synthetic G.711 WAVE, Ogg Opus, bare FLAC, multitrack Matroska (PCM + FLAC), and multitrack QuickTime (PCM + IMA4 ADPCM) trim, loudness, and constant-gain normalization regressions for FVID-0211."""

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
OUT = ROOT / "tests/fixtures/playback-errors/media-trim-loudness"


def u(tag: str, n: int) -> bytes:
    return ebml(tag, n.to_bytes(max(1, (n.bit_length() + 7) // 8), "big"))


def simple_block(track_no: int, rel_ms: int, flags: int, payload: bytes) -> bytes:
    assert 1 <= track_no <= 126
    return ebml("a3", bytes([0x80 | track_no]) + struct.pack(">hB", rel_ms, flags) + payload)


def box(kind: bytes, data: bytes) -> bytes:
    return struct.pack(">I4s", len(data) + 8, kind) + data


def build_raw_flac_48k() -> bytes:
    """Bare FLAC stream (16-bit 48 kHz mono) with 11 packets of 1920 samples (440 ms total, exceeding the 400 ms loudness gating window)."""
    flac_packets = [
        flac_frame(i, 1920, False, 8192 if i % 2 == 0 else -8192, block_code=7, rate_code=10)
        for i in range(11)
    ]
    return flac_stream(flac_packets, [1920] * 11, rate=48_000)


def build_multitrack_pcm_flac_mka() -> bytes:
    """Matroska audio container with Track 1 = A_PCM/INT/LIT (16-bit 48 kHz mono) and Track 2 = A_FLAC (16-bit 48 kHz mono).
    Contains 11 packets of 1920 samples (440 ms total, exceeding the 400 ms loudness gating window).
    """
    flac_packets = [
        flac_frame(i, 1920, False, 8192 if i % 2 == 0 else -8192, block_code=7, rate_code=10)
        for i in range(11)
    ]
    raw_flac = flac_stream(flac_packets, [1920] * 11, rate=48_000)
    streaminfo = raw_flac[8:42]
    flac_private = b"fLaC\x80\x00\x00\x22" + streaminfo

    header = ebml("1a45dfa3", ebml("4282", b"matroska") + u("4287", 4) + u("4285", 2))
    info = ebml("1549a966", u("2ad7b1", 1_000_000))
    audio_settings = ebml("e1", ebml("b5", struct.pack(">d", 48000.0)) + u("9f", 1) + u("6264", 16))
    track1 = ebml(
        "ae",
        u("d7", 1)
        + u("73c5", 101)
        + u("83", 2)
        + ebml("86", b"A_PCM/INT/LIT")
        + audio_settings,
    )
    track2 = ebml(
        "ae",
        u("d7", 2)
        + u("73c5", 102)
        + u("83", 2)
        + ebml("86", b"A_FLAC")
        + ebml("63a2", flac_private)
        + audio_settings,
    )
    tracks = ebml("1654ae6b", track1 + track2)
    blocks = u("e7", 0)
    for i, flac_pkt in enumerate(flac_packets):
        stamp = i * 40  # 1920 / 48000 = 40 ms
        pcm_val = 4096 if i % 2 == 0 else -4096
        pcm_pkt = struct.pack("<h", pcm_val) * 1920
        blocks += simple_block(1, stamp, 0x80, pcm_pkt)
        blocks += simple_block(2, stamp, 0x80, flac_pkt)
    return header + ebml("18538067", info + tracks + ebml("1f43b675", blocks))


def build_multitrack_pcm_ima4_mov() -> bytes:
    """QuickTime/MP4 container with Track 1 = sowt (16-bit 48 kHz mono PCM, 21120 samples)
    and Track 2 = ima4 (IMA4 ADPCM 48 kHz mono, 330 packets * 64 samples = 21120 samples = 440 ms).
    """
    packets = 330
    samples_per_packet = 64
    total_samples = packets * samples_per_packet  # 21120 samples = 440 ms > 400 ms loudness gate
    sowt_payload = b"".join(
        struct.pack("<h", 4096 if (i // 32) % 2 == 0 else -4096) for i in range(total_samples)
    )
    # IMA4 ADPCM 34-byte packet: 2-byte predictor/step header + 32 bytes of nibbles
    # Header: predictor = +4096 (0x1000) or -4096 (0xF000), step_index = 20 -> (pred & 0xFF80) | step_index
    ima4_pkt_pos = struct.pack(">H", (4096 & 0xFF80) | 20) + bytes([0x08] * 32)
    ima4_pkt_neg = struct.pack(">H", ((-4096) & 0xFF80) | 20) + bytes([0x08] * 32)
    ima4_payload = b"".join(ima4_pkt_pos if i % 2 == 0 else ima4_pkt_neg for i in range(packets))
    mdat_body = sowt_payload + ima4_payload
    mdat = box(b"mdat", mdat_body)

    def make_trak(track_id: int, codec: bytes, offset: int, pkt_count: int, pkt_samples: int, pkt_size: int) -> bytes:
        clock = bytearray(24)
        struct.pack_into(">II", clock, 12, 48_000, total_samples)
        entry = bytearray(28)
        struct.pack_into(">H", entry, 6, 1)
        struct.pack_into(">HH", entry, 16, 1, 16)
        struct.pack_into(">I", entry, 24, 48_000 << 16)
        entry += box(b"enda", struct.pack(">H", 1))
        stsd = box(b"stsd", struct.pack(">II", 0, 1) + box(codec, bytes(entry)))
        stts = box(b"stts", struct.pack(">IIII", 0, 1, pkt_count, pkt_samples))
        stsc = box(b"stsc", struct.pack(">IIIII", 0, 1, 1, pkt_count, 1))
        stsz = box(b"stsz", struct.pack(">III", 0, pkt_size, pkt_count))
        stco = box(b"stco", struct.pack(">III", 0, 1, offset))
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
        struct.pack_into(">I", tkhd, 12, track_id)
        struct.pack_into(">I", tkhd, 20, total_samples)
        for off, val in [(40, 1 << 16), (56, 1 << 16), (72, 1 << 30)]:
            struct.pack_into(">I", tkhd, off, val)
        return box(b"trak", box(b"tkhd", bytes(tkhd)) + mdia)

    trak1 = make_trak(1, b"sowt", 8, 1, total_samples, len(sowt_payload))
    trak2 = make_trak(2, b"ima4", 8 + len(sowt_payload), packets, samples_per_packet, 34)
    mvhd = bytearray(100)
    struct.pack_into(">II", mvhd, 12, 48_000, total_samples)
    struct.pack_into(">IH", mvhd, 20, 1 << 16, 256)
    for off, val in [(36, 1 << 16), (52, 1 << 16), (68, 1 << 30), (96, 3)]:
        struct.pack_into(">I", mvhd, off, val)
    return mdat + box(b"moov", box(b"mvhd", bytes(mvhd)) + trak1 + trak2)


def generate() -> dict[str, bytes]:
    # 21,600 samples at 48 kHz = 450 ms (> 400 ms loudness gating window)
    # 0x80 / 0x00 in A-law decode to +/-5504 (~-18.3 LUFS) so gain to -23 LUFS is within (0, 64]
    codes_alaw = bytes((0x80 if (i % 16 < 8) else 0x00) for i in range(21_600))
    codes_mulaw = bytes((0x1F if (i % 16 < 8) else 0x9F) for i in range(21_600))
    g711_alaw = make_g711_wave(law="alaw", sample_rate=48_000, channels=1, codes=codes_alaw)
    g711_mulaw = make_g711_wave(law="mulaw", sample_rate=48_000, channels=1, codes=codes_mulaw)

    opus_head = b"OpusHead" + bytes([1, 1]) + struct.pack("<HIhB", 0, 48_000, 0, 0)
    opus_tags = b"OpusTags" + struct.pack("<I", 4) + b"fvid" + struct.pack("<I", 0)
    raw20 = bytes.fromhex("f8fffe")
    ogg_opus = b"".join(
        [
            ogg_page(opus_head, 0, 2, 0, 0x46560211),
            ogg_page(opus_tags, 0, 0, 1, 0x46560211),
            ogg_page(raw20, 960, 0, 2, 0x46560211),
            ogg_page(raw20, 1920, 4, 3, 0x46560211),
        ]
    )

    files = {
        "video.mkv": owned_eof()["video.mkv"],
        "g711-alaw-48k-450ms.wav": g711_alaw,
        "g711-mulaw-48k-450ms.wav": g711_mulaw,
        "ogg-opus-mono-48k.ogg": ogg_opus,
        "raw-flac-48k-440ms.flac": build_raw_flac_48k(),
        "multitrack-pcm-flac.mka": build_multitrack_pcm_flac_mka(),
        "multitrack-pcm-ima4.mov": build_multitrack_pcm_ima4_mov(),
    }
    manifest = {
        "batch_id": "FVID-0211",
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
