#!/usr/bin/env python3
"""Generate synthetic fixtures for Batch 583 (FVID-0645..0649):
1. ogg-opus-two-packet.ogg: Ogg Opus stream (48 kHz stereo) with 2 audio packets
   (0..960 and 960..1920 samples = 0..20 ms and 20..40 ms), used to test
   max_packets-bounded interval prefix validation.
2. ogg-vorbis-six-packet.ogg: Ogg Vorbis stream (48 kHz stereo) with 6 audio
   packets (ends at 0, 80, 160, 240, 368, 411 samples), used to test
   max_packets-bounded interval prefix validation.
3. ogg-joint-vorbis-opus.ogg: Linked Ogg Vorbis (411 samples) -> Ogg Opus (1920
   samples) stream, used to test max_packets-bounded interval prefix validation
   across linked Ogg programs.
4. g711-alaw-multi-packet.wav: G.711 A-law WAVE file at 8000 Hz mono with 16384
   samples (8 packet windows of 2048 frames), used to test max_packets-bounded
   spool budget in apply and actual frame count in plan_decode_audio.
"""

from pathlib import Path
import struct

from generate_wave_ogg_flac_plan_regressions import (
    make_g711_wave,
    make_opus_link,
    make_vorbis_link,
)

OUT_DIR = (
    Path(__file__).resolve().parent.parent
    / "tests"
    / "fixtures"
    / "playback-errors"
    / "ogg-packet-limit-interval-and-g711-budget"
)


def main() -> None:
    OUT_DIR.mkdir(parents=True, exist_ok=True)
    opus = make_opus_link(serial=0x46560645, channels=2)
    vorbis = make_vorbis_link(serial=0x46560646, channels=2, sample_rate=48_000)
    joint = (
        make_vorbis_link(serial=0x46560647, channels=2, sample_rate=48_000)
        + make_opus_link(serial=0x46560648, channels=2)
    )
    codes = bytes((0x55 if (i & 8) == 0 else 0xD5) for i in range(16384))
    g711 = make_g711_wave(law="alaw", sample_rate=8_000, channels=1, codes=codes)

    (OUT_DIR / "ogg-opus-two-packet.ogg").write_bytes(opus)
    (OUT_DIR / "ogg-vorbis-six-packet.ogg").write_bytes(vorbis)
    (OUT_DIR / "ogg-joint-vorbis-opus.ogg").write_bytes(joint)
    (OUT_DIR / "g711-alaw-multi-packet.wav").write_bytes(g711)


if __name__ == "__main__":
    main()
