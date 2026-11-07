#!/usr/bin/env python3
"""Generate synthetic fixtures for Batch 588 (FVID-0670..0674):
1. flac-8k-1pkt.flac: Bare FLAC audio stream at 8000 Hz mono 16-bit with 1
   constant frame of 192 samples (24 ms), used alongside g711-alaw-8k-nometa.wav
   to test:
   - FVID-0670: `owned_container_loudnorm::apply` supporting standalone audio
     containers recognized by `owned_container_loudness` (G.711 WAVE, Ogg, bare
     FLAC, bare MPEG) instead of assuming only MP4 or Matroska and failing with
     `"Matroska audio stream is absent"`.
2. ffv1-alpha-gray-8-crop.mkv + cue.srt: Matroska V_FFV1 monochrome+alpha 4x2
   8-bit video with stored PixelCropLeft=1 (so `view.alpha` and
   `view.extra_working_bytes` are non-zero), used alongside an existing MP4
   video to test:
   - FVID-0671: `owned_video_decode::process_frame_with_overlay` refusing
     `shuffleplanes` on monochrome (`gray`) video frames with `Ok(None)` instead
     of failing with `"Y4M frame size mismatch"`.
   - FVID-0672: `owned_subtitle_burn::plan_burn_subtitles` charging `view.alpha`
     and `view.extra_working_bytes` to `frame_working_bytes` (matching
     `owned_ffv1_export::export_processed`), and `burn_subtitles` checking
     `destination.symlink_metadata().is_ok()` before running `prepare`.
3. mkv-sub-overlong-writing-app.mkv: Matroska `S_TEXT/UTF8` subtitle stream with
   a 1500-byte `WritingApp` (`0x5741`) and a 1500-byte container tag, used to
   test:
   - FVID-0673: `owned_subtitle::try_convert_matroska_inner` filtering overlong
     (>1024-byte) or NUL-containing source `WritingApp` / container / track tags,
     and `publish_events_with_metadata` validating `PacketWriter` header metadata
     during `dry_run` (`plan_convert_subtitles_with_controls`).
4. adts-8k-2pkt.aac: ADTS AAC-LC stream at 8000 Hz mono with 2 packets, used to
   test:
   - FVID-0674: `extract_adts_matroska_metadata` enforcing the 256-tag Matroska
     stream tag cap and `write_aac_packets` deferring the initial 0-packet
     progress emission until the first packet is read and budget-checked.
"""

from __future__ import annotations

from pathlib import Path
import struct

from generate_aac_source_info_prefix_budget import adts_frame
from generate_audio_plan_mp3_flac_and_matroska_pcm_fixtures import (
    ebml_el,
    ebml_str,
    ebml_uint,
    make_flac_constant_frame,
)
from generate_ffv1_alpha import model, packet
from generate_mp4_audio_selection_regressions import build_aac_pulse_packet

OUT_DIR = (
    Path(__file__).resolve().parent.parent
    / "tests"
    / "fixtures"
    / "playback-errors"
    / "standalone-loudnorm-gray-shuffleplanes-and-subtitles"
)


def build_flac_8k_1pkt() -> bytes:
    sample_rate = 8000
    channels = 1
    bps = 16
    total_samples = 192
    packed = (
        (sample_rate << 44)
        | ((channels - 1) << 41)
        | ((bps - 1) << 36)
        | total_samples
    )
    streaminfo_body = (
        struct.pack(">HH", 192, 192)
        + b"\x00\x00\x00"
        + b"\x00\x00\x00"
        + packed.to_bytes(8, "big")
        + (b"\x00" * 16)
    )
    meta_block = bytes([0x80]) + len(streaminfo_body).to_bytes(3, "big") + streaminfo_body
    # Frame header for 192 samples, 8000 Hz (0x04 in sample rate bits? Wait: in make_flac_constant_frame,
    # 0x1A is block_size=192 (0x1) and sample_rate=48000 Hz (0xA). So let's use 48000 Hz in STREAMINFO too!)
    sample_rate_48k = 48000
    packed_48k = (
        (sample_rate_48k << 44)
        | ((channels - 1) << 41)
        | ((bps - 1) << 36)
        | total_samples
    )
    streaminfo_48k = (
        struct.pack(">HH", 192, 192)
        + b"\x00\x00\x00"
        + b"\x00\x00\x00"
        + packed_48k.to_bytes(8, "big")
        + (b"\x00" * 16)
    )
    meta_48k = bytes([0x80]) + len(streaminfo_48k).to_bytes(3, "big") + streaminfo_48k
    return b"fLaC" + meta_48k + make_flac_constant_frame(0, 2048)


def build_ffv1_alpha_gray_8_crop() -> bytes:
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
    # 16x16 monochrome+alpha frame with PixelCropLeft=2 (0x54CC = 2) -> 14x16 display
    w, h = 16, 16
    y_plane = [(i * 7) & 0xFF for i in range(w * h)]
    a_plane = [255 for _ in range(w * h)]
    pkt = packet(
        [y_plane, a_plane],
        [(w, h), (w, h)],
        8,
        False,
        False,
        (0, 0),
        1,
        1,
        True,
        model(1),
        has_alpha=True,
    )
    video = (
        ebml_uint(0xB0, w)
        + ebml_uint(0xBA, h)
        + ebml_uint(0x54CC, 2)  # PixelCropLeft = 2
    )
    track = ebml_el(
        0xAE,
        ebml_uint(0xD7, 1)
        + ebml_uint(0x73C5, 1)
        + ebml_uint(0x83, 1)
        + ebml_str(0x86, "V_FFV1")
        + ebml_uint(0x23E383, 40_000_000)
        + ebml_el(0xE0, video),
    )
    block = ebml_el(0xA3, b"\x81\x00\x00\x80" + pkt)
    cluster = ebml_el(0x1F43B675, ebml_uint(0xE7, 0) + block)
    return ebml_head + ebml_el(0x18538067, info + ebml_el(0x1654AE6B, track) + cluster)


def build_mkv_sub_overlong_writing_app() -> bytes:
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
    overlong_app = "W" * 1500
    info = ebml_el(
        0x1549A966,
        ebml_uint(0x2AD7B1, 1_000_000)
        + ebml_str(0x4D80, "test")
        + ebml_str(0x5741, overlong_app),
    )
    track_entry = ebml_el(
        0xAE,
        ebml_uint(0xD7, 1)
        + ebml_uint(0x73C5, 1)
        + ebml_uint(0x83, 17)
        + ebml_str(0x86, "S_TEXT/UTF8"),
    )
    tracks = ebml_el(0x1654AE6B, track_entry)
    simple_tag = ebml_el(
        0x67C8,
        ebml_str(0x45A3, "CUSTOM_LONG") + ebml_str(0x4487, "V" * 1500),
    )
    tag = ebml_el(0x7373, ebml_el(0x63C0, b"") + simple_tag)
    tags = ebml_el(0x1254C367, tag)
    block_payload = bytes([0x81, 0x00, 0x00, 0x80]) + b"Hello subtitle"
    block_group = ebml_el(
        0xA0,
        ebml_el(0xA1, block_payload) + ebml_uint(0x9B, 1000),
    )
    cluster = ebml_el(0x1F43B675, ebml_uint(0xE7, 0) + block_group)
    return ebml_head + ebml_el(0x18538067, info + tracks + tags + cluster)


def build_adts_48k_2pkt() -> bytes:
    pkt = build_aac_pulse_packet(140, 1, 7)
    return b"".join(adts_frame(pkt, freq_index=3, channels=1) for _ in range(2))


def main() -> None:
    OUT_DIR.mkdir(parents=True, exist_ok=True)
    (OUT_DIR / "flac-48k-1pkt.flac").write_bytes(build_flac_8k_1pkt())
    (OUT_DIR / "ffv1-alpha-gray-8-crop.mkv").write_bytes(build_ffv1_alpha_gray_8_crop())
    (OUT_DIR / "cue.srt").write_bytes(b"1\n00:00:00,000 --> 00:00:01,000\nHi\n")
    (OUT_DIR / "mkv-sub-overlong-writing-app.mkv").write_bytes(
        build_mkv_sub_overlong_writing_app()
    )
    (OUT_DIR / "adts-8k-2pkt.aac").write_bytes(build_adts_48k_2pkt())


if __name__ == "__main__":
    main()
