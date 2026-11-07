#!/usr/bin/env python3
"""Generate synthetic fixtures for Batch 586 (FVID-0660..0664):
1. g711-alaw-8k-meta.wav: G.711 A-law WAVE file at 8000 Hz mono with 8000
   sample frames (1.0 s = 4 packet windows of 2048/1856 frames) and a 4000-byte
   RIFF LIST/INFO INAM chunk, used to test:
   - FVID-0660: owned_container_loudness::check_meter_budget accounting for
     integrated_blocks + short_blocks histogram bin storage.
   - FVID-0662: owned_g711_wave_export::descriptor_for_decode reporting RIFF
     INFO metadata length into source_descriptor_for_decode so plan_decode_audio
     and owned_container_loudnorm enforce Stage-2 metadata working memory.
2. adts-lc-48k-30pkt.aac: ADTS AAC-LC stream at 48000 Hz stereo with 45 packets
   (46080 sample frames), used to test:
   - FVID-0661: adts_prefix_info_for_transform computing exact prefix sample
     frames under max_controlled_bytes when transform is None, and
     owned_adts_loudnorm::plan_loudnorm / apply enforcing Stage-2 decoded spool
     frame count.
   - FVID-0664: owned_audio_mix::audio_plan_with_controls including ADTS input
     frames in the mix/merge output buffer budget check.
3. mka-pcm-tagged.mka: Matroska audio file (A_PCM/INT/LIT, 8000 Hz mono, 1024
   samples) with a 4000-byte Track Name tag, used to test:
   - FVID-0663: owned_audio_plan::source_descriptor_for_decode populating
     stage2_meta_bytes for Matroska/MP4 audio inputs in plan_decode_audio and
     container loudnorm.
"""

from __future__ import annotations

from pathlib import Path
import struct

from generate_aac_source_info_prefix_budget import adts_frame
from generate_audio_plan_mp3_flac_and_matroska_pcm_fixtures import (
    ebml_el,
    ebml_str,
    ebml_uint,
)
from generate_mp4_audio_selection_regressions import build_aac_pulse_packet
from generate_wave_ogg_flac_plan_regressions import riff_chunk

OUT_DIR = (
    Path(__file__).resolve().parent.parent
    / "tests"
    / "fixtures"
    / "playback-errors"
    / "loudness-histogram-adts-and-spool-meta"
)


def build_g711_alaw_8k_meta() -> bytes:
    sample_rate = 8000
    channels = 1
    codes = bytes((0x55 if (i & 8) == 0 else 0xD5) for i in range(8000))
    fmt = struct.pack(
        "<HHIIHHH", 6, channels, sample_rate, sample_rate * channels, channels, 8, 0
    )
    title_bytes = (b"G711Meta" * 500) + b"\x00"
    info = riff_chunk(b"LIST", b"INFO" + riff_chunk(b"INAM", title_bytes))
    body = (
        b"WAVE"
        + riff_chunk(b"fmt ", fmt)
        + riff_chunk(b"fact", struct.pack("<I", len(codes)))
        + info
        + riff_chunk(b"data", codes)
    )
    return b"RIFF" + struct.pack("<I", len(body)) + body


def build_adts_lc_48k_30pkt() -> bytes:
    pkt = build_aac_pulse_packet(140, 1, 7)
    return b"".join(adts_frame(pkt, freq_index=3, channels=1) for _ in range(250))


def build_mka_pcm_tagged() -> bytes:
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
    info = ebml_el(
        0x1549A966,
        ebml_uint(0x2AD7B1, 1_000_000) + ebml_str(0x5741, "FVidTestMuxer" * 70),
    )
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
        + ebml_str(0x536E, "MkaTrackMeta" * 80)
        + ebml_str(0x86, "A_PCM/INT/LIT")
        + audio_settings,
    )
    tracks = ebml_el(0x1654AE6B, track_entry)
    simple_tags = (
        ebml_el(0x67C8, ebml_str(0x45A3, "ARTIST") + ebml_str(0x4487, "MkaArtist" * 100))
        + ebml_el(0x67C8, ebml_str(0x45A3, "COMMENT") + ebml_str(0x4487, "MkaComment" * 90))
        + ebml_el(0x67C8, ebml_str(0x45A3, "ALBUM") + ebml_str(0x4487, "MkaAlbum" * 100))
    )
    tags = ebml_el(0x1254C367, ebml_el(0x7373, ebml_el(0x63C0, b"") + simple_tags))
    pcm_payload = b"".join(
        struct.pack("<h", 4096 if (i % 16) < 8 else -4096) for i in range(1024)
    )
    simple_block = ebml_el(0xA3, b"\x81\x00\x00\x80" + pcm_payload)
    cluster = ebml_el(0x1F43B675, ebml_uint(0xE7, 0) + simple_block)
    segment = ebml_el(0x18538067, info + tracks + tags + cluster)
    return ebml_head + segment


def main() -> None:
    OUT_DIR.mkdir(parents=True, exist_ok=True)
    (OUT_DIR / "g711-alaw-8k-meta.wav").write_bytes(build_g711_alaw_8k_meta())
    (OUT_DIR / "adts-lc-48k-30pkt.aac").write_bytes(build_adts_lc_48k_30pkt())
    (OUT_DIR / "mka-pcm-tagged.mka").write_bytes(build_mka_pcm_tagged())


if __name__ == "__main__":
    main()
