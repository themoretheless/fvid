#!/usr/bin/env python3
"""Generate synthetic fixtures for Batch 585 (FVID-0655..0659):
1. ogg-opus-mix-prefix.ogg: Ogg Opus stream (48 kHz mono) with 50 audio packets
   (48,000 samples = 1.0 s), used to test:
   - FVID-0655: plan_mix_audio_with_controls and plan_merge_audio_with_controls
     bounding Ogg input sample frame estimates by controls.max_packets.
   - FVID-0656: probe_ogg_with_options counting audio packets (50) rather than
     all Ogg framing packets including OpusHead + OpusTags (52).
   - FVID-0659: check_spool_loudnorm_budget accounting for actual decoded prefix
     sample frames in container plan_loudnorm and apply_loudnorm.
2. mp3-info-probe-prefix.mp3: Bare MPEG-1 Layer III stream (44,100 Hz mono) with
   a leading LAME Info frame and 4 coded frames where frame 3 is trimmed by
   end_padding (3 active audio frames + 1 Info packet = 4 packet work units),
   used to test FVID-0657 in probe_mpeg_with_options.
3. mka-untitled-chapter.mka: Matroska audio file (A_PCM/INT/LIT, 48 kHz mono)
   with 2 chapters (Chapter 0 untitled, Chapter 1 titled "Act II"), used to test
   FVID-0658 in fvid::media_probe::probe_with_options omitting empty "title".
"""

from pathlib import Path
import struct

from generate_audio_plan_mp3_flac_and_matroska_pcm_fixtures import (
    build_mp3_info_prefix_interval,
    ebml_el,
    ebml_str,
    ebml_uint,
)
from generate_flac_ogg_info_cap_and_spool_budget_fixtures import (
    build_ogg_opus_stream,
)

OUT_DIR = (
    Path(__file__).resolve().parent.parent
    / "tests"
    / "fixtures"
    / "playback-errors"
    / "mix-probe-chapter-and-loudnorm-budget"
)


def build_mka_untitled_chapter() -> bytes:
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
        ebml_uint(0x2AD7B1, 1_000_000) + ebml_str(0x5741, "FVidTestMuxer"),
    )
    audio_settings = ebml_el(
        0xE1,
        ebml_el(0xB5, struct.pack(">d", 48000.0))
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

    # Chapter 0: untitled (no ChapterDisplay 0x80), 0..20_000_000 ns
    atom0 = ebml_el(
        0xB6,
        ebml_uint(0x73C4, 1)
        + ebml_uint(0x91, 0)
        + ebml_uint(0x92, 20_000_000),
    )
    # Chapter 1: titled "Act II", 20_000_000..40_000_000 ns
    atom1 = ebml_el(
        0xB6,
        ebml_uint(0x73C4, 2)
        + ebml_uint(0x91, 20_000_000)
        + ebml_uint(0x92, 40_000_000)
        + ebml_el(0x80, ebml_str(0x85, "Act II")),
    )
    chapters = ebml_el(0x1043A770, ebml_el(0x45B9, atom0 + atom1))

    pcm_payload = b"".join(
        struct.pack("<h", 4096 if (i % 32) < 16 else -4096) for i in range(1920)
    )
    simple_block = ebml_el(0xA3, b"\x81\x00\x00\x80" + pcm_payload)
    cluster = ebml_el(0x1F43B675, ebml_uint(0xE7, 0) + simple_block)
    segment = ebml_el(0x18538067, info + tracks + chapters + cluster)
    return ebml_head + segment


def main() -> None:
    OUT_DIR.mkdir(parents=True, exist_ok=True)
    (OUT_DIR / "ogg-opus-mix-prefix.ogg").write_bytes(
        build_ogg_opus_stream(serial=0x585001, channels=2, num_packets=150)
    )
    (OUT_DIR / "mp3-info-probe-prefix.mp3").write_bytes(
        build_mp3_info_prefix_interval()
    )
    (OUT_DIR / "mka-untitled-chapter.mka").write_bytes(
        build_mka_untitled_chapter()
    )


if __name__ == "__main__":
    main()
