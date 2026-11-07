#!/usr/bin/env python3
"""Generate synthetic fixtures for Batch 587 (FVID-0665..0669):
1. mp4-sowt-8k-8pkt.m4a: MP4 audio file (`sowt` 16-bit PCM, 8000 Hz mono,
   8 packets of 1000 samples = 8000 sample frames = 1.0 s), used to test:
   - FVID-0665: `owned_mp4_audio_export::geometry_and_frames_for_decode` and
     `owned_audio_plan::source_descriptor_for_decode` populating `stage2_frames`
     for MP4 audio inputs so Stage-2 WAVE export (`check_spool_wave_export_budget`),
     container loudness (`owned_container_loudness::check_meter_budget` histogram
     bins), and container loudnorm (`owned_container_loudnorm::check_spool_loudnorm_budget`)
     charge the actual MP4 presentation frame count instead of 1024.
   - FVID-0667: `owned_audio_mix::audio_plan_with_controls` including MP4 audio
     inputs in `est_frames` so `plan_mix_audio_with_controls` and
     `plan_merge_audio_with_controls` enforce the mix/merge output buffer budget.
2. mka-pcm-8k-8pkt.mka: Matroska audio file (`A_PCM/INT/LIT` 16-bit PCM,
   8000 Hz mono, 8 SimpleBlocks of 1000 samples = 8000 sample frames = 1.0 s),
   used to test:
   - FVID-0666: `owned_matroska_audio::matroska_estimated_frames` and
     `owned_audio_plan::matroska_descriptor_for_decode` populating `stage2_frames`
     for Matroska audio inputs across Stage-2 WAVE export, container loudness
     histogram bins, and container loudnorm.
   - FVID-0667: `owned_audio_mix::audio_plan_with_controls` including Matroska
     audio inputs in `est_frames`.
3. g711-alaw-8k-nometa.wav: G.711 A-law WAVE file at 8000 Hz mono with 8000
   sample frames (1.0 s) and no RIFF INFO metadata (`metadata_bytes == 0`), used
   to test:
   - FVID-0668: `owned_adts_export::check_spool_wave_export_budget` enforcing
     `estimate_float_wave_export_bytes` even when `transform` is identity and
     `metadata_bytes == 0`.
   - FVID-0669: `owned_audio_mix::audio_plan_with_controls` calling
     `plan_decode_audio` on G.711 WAVE inputs so `plan_mix_audio_with_controls`
     and `plan_merge_audio_with_controls` enforce the Stage-2 WAVE decode budget
     before decoding to a temporary spool.
"""

from __future__ import annotations

from pathlib import Path
import struct

from generate_audio_plan_mp3_flac_and_matroska_pcm_fixtures import (
    ebml_el,
    ebml_str,
    ebml_uint,
)
from generate_mp4_audio_selection_regressions import make_audio_track, sowt_v0_header
from generate_vp9_chroma import atom, word
from generate_wave_ogg_flac_plan_regressions import riff_chunk

OUT_DIR = (
    Path(__file__).resolve().parent.parent
    / "tests"
    / "fixtures"
    / "playback-errors"
    / "container-stage2-frames-and-spool-budget"
)


def build_mp4_sowt_8k_8pkt() -> bytes:
    rate = 8000
    delta = 1000
    pkt = b"".join(
        struct.pack("<h", 4096 if (i % 16) < 8 else -4096) for i in range(delta)
    )
    packets = [pkt for _ in range(8)]
    payload = b"".join(packets)
    ftyp = atom(b"ftyp", b"M4A \x00\x00\x02\x00M4A isommp42")
    mdat = atom(b"mdat", payload)
    offset = len(ftyp) + 8
    trak = make_audio_track(
        track_id=1,
        flags=3,
        codec=b"sowt",
        entry_payload=sowt_v0_header(1, rate),
        packets=packets,
        offset=offset,
        rate=rate,
        delta=delta,
    )
    mvhd = bytearray(100)
    mvhd[12:16] = word(rate)
    mvhd[16:20] = word(len(packets) * delta)
    mvhd[20:24] = word(65536)
    mvhd[24:28] = word(0x01000000)
    for at, val in [(36, 65536), (52, 65536), (68, 1 << 30), (96, 2)]:
        mvhd[at : at + 4] = word(val)
    moov = atom(b"moov", atom(b"mvhd", bytes(mvhd)) + trak)
    return ftyp + mdat + moov


def build_mka_pcm_8k_8pkt() -> bytes:
    ebml_head = ebml_el(
        0x1A45DFA3,
        ebml_uint(0x4286, 1)
        + ebml_uint(0x42F7, 1)
        + ebml_uint(0x42F2, 4)
        + ebml_uint(0x42F3, 8)
        + ebml_str(0x4282, "matroska")
        + ebml_uint(0x4287, 4)
        + ebml_uint(0x4285, 2),
    )
    info = ebml_el(0x1549A966, ebml_uint(0x2AD7B1, 1_000_000))
    audio_settings = ebml_el(
        0xE1,
        ebml_el(0xB5, struct.pack(">d", 8000.0))
        + ebml_uint(0x9F, 1)
        + ebml_uint(0x6264, 16),
    )
    track_entry = ebml_el(
        0xAE,
        ebml_uint(0xD7, 1)
        + ebml_uint(0x73C5, 1)
        + ebml_uint(0x83, 2)
        + ebml_str(0x86, "A_PCM/INT/LIT")
        + audio_settings,
    )
    tracks = ebml_el(0x1654AE6B, track_entry)
    pcm_payload = b"".join(
        struct.pack("<h", 4096 if (i % 16) < 8 else -4096) for i in range(1000)
    )
    blocks = b"".join(
        ebml_el(0xA3, b"\x81" + struct.pack(">h", idx * 125) + b"\x80" + pcm_payload)
        for idx in range(8)
    )
    cluster = ebml_el(0x1F43B675, ebml_uint(0xE7, 0) + blocks)
    segment = ebml_el(0x18538067, info + tracks + cluster)
    return ebml_head + segment


def build_g711_alaw_8k_nometa() -> bytes:
    sample_rate = 8000
    channels = 1
    codes = bytes((0x55 if (i & 8) == 0 else 0xD5) for i in range(20000))
    fmt = struct.pack(
        "<HHIIHHH", 6, channels, sample_rate, sample_rate * channels, channels, 8, 0
    )
    body = (
        b"WAVE"
        + riff_chunk(b"fmt ", fmt)
        + riff_chunk(b"fact", struct.pack("<I", len(codes)))
        + riff_chunk(b"data", codes)
    )
    return b"RIFF" + struct.pack("<I", len(body)) + body


def build_mp4_ima4_8k_125pkt() -> bytes:
    rate = 8000
    delta = 64
    pkt = b"\x00\x00" + b"\x00" * 32
    packets = [pkt for _ in range(125)]
    payload = b"".join(packets)
    ftyp = atom(b"ftyp", b"M4A \x00\x00\x02\x00M4A isommp42")
    mdat = atom(b"mdat", payload)
    offset = len(ftyp) + 8
    trak = make_audio_track(
        track_id=1,
        flags=3,
        codec=b"ima4",
        entry_payload=sowt_v0_header(1, rate),
        packets=packets,
        offset=offset,
        rate=rate,
        delta=delta,
    )
    mvhd = bytearray(100)
    mvhd[12:16] = word(rate)
    mvhd[16:20] = word(len(packets) * delta)
    mvhd[20:24] = word(65536)
    mvhd[24:28] = word(0x01000000)
    for at, val in [(36, 65536), (52, 65536), (68, 1 << 30), (96, 2)]:
        mvhd[at : at + 4] = word(val)
    moov = atom(b"moov", atom(b"mvhd", bytes(mvhd)) + trak)
    return ftyp + mdat + moov


def build_mka_mp3_44k_8pkt() -> bytes:
    ebml_head = ebml_el(
        0x1A45DFA3,
        ebml_uint(0x4286, 1)
        + ebml_uint(0x42F7, 1)
        + ebml_uint(0x42F2, 4)
        + ebml_uint(0x42F3, 8)
        + ebml_str(0x4282, "matroska")
        + ebml_uint(0x4287, 4)
        + ebml_uint(0x4285, 2),
    )
    info = ebml_el(0x1549A966, ebml_uint(0x2AD7B1, 1_000_000))
    audio_settings = ebml_el(
        0xE1,
        ebml_el(0xB5, struct.pack(">d", 44100.0)) + ebml_uint(0x9F, 1),
    )
    track_entry = ebml_el(
        0xAE,
        ebml_uint(0xD7, 1)
        + ebml_uint(0x73C5, 1)
        + ebml_uint(0x83, 2)
        + ebml_uint(0x23E383, round(1152 * 1_000_000_000 / 44100))
        + ebml_str(0x86, "A_MPEG/L3")
        + audio_settings,
    )
    tracks = ebml_el(0x1654AE6B, track_entry)
    mp3_frame = bytes([0xFF, 0xFB, 0x10, 0xC4]) + b"\x00" * 100
    blocks = b"".join(
        ebml_el(
            0xA3,
            b"\x81" + struct.pack(">h", round(idx * 1152 * 1000 / 44100)) + b"\x80" + mp3_frame,
        )
        for idx in range(8)
    )
    cluster = ebml_el(0x1F43B675, ebml_uint(0xE7, 0) + blocks)
    segment = ebml_el(0x18538067, info + tracks + cluster)
    return ebml_head + segment


def main() -> None:
    OUT_DIR.mkdir(parents=True, exist_ok=True)
    (OUT_DIR / "mp4-sowt-8k-8pkt.m4a").write_bytes(build_mp4_sowt_8k_8pkt())
    (OUT_DIR / "mka-pcm-8k-8pkt.mka").write_bytes(build_mka_pcm_8k_8pkt())
    (OUT_DIR / "mp4-ima4-8k-125pkt.m4a").write_bytes(build_mp4_ima4_8k_125pkt())
    (OUT_DIR / "mka-mp3-44k-8pkt.mka").write_bytes(build_mka_mp3_44k_8pkt())
    (OUT_DIR / "g711-alaw-8k-nometa.wav").write_bytes(build_g711_alaw_8k_nometa())


if __name__ == "__main__":
    main()
