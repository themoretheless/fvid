#!/usr/bin/env python3
"""Generate synthetic G.711 WAVE, Ogg Opus, bare FLAC, multitrack Matroska (PCM + FLAC), and multitrack QuickTime (PCM + IMA4 ADPCM) fixtures for .f32le/.mka/.mkv decode-audio, trim, and normalize-loudness regressions (FVID-0213)."""

from __future__ import annotations

import hashlib
import json
from pathlib import Path
import struct

from generate_audio_reader_eof_regressions import generate as owned_eof
from generate_media_trim_loudness_regressions import (
    build_multitrack_pcm_flac_mka,
    build_multitrack_pcm_ima4_mov,
    build_raw_flac_48k,
)
from generate_wave_ogg_flac_plan_regressions import make_g711_wave, ogg_page


ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / "tests/fixtures/playback-errors/owned-audio-f32le-matroska"


def generate() -> dict[str, bytes]:
    # 21,600 samples at 48 kHz = 450 ms (> 400 ms loudness gating window)
    codes_alaw = bytes((0x80 if (i % 16 < 8) else 0x00) for i in range(21_600))
    codes_mulaw = bytes((0x1F if (i % 16 < 8) else 0x9F) for i in range(21_600))
    g711_alaw = make_g711_wave(law="alaw", sample_rate=48_000, channels=1, codes=codes_alaw)
    g711_mulaw = make_g711_wave(law="mulaw", sample_rate=48_000, channels=1, codes=codes_mulaw)

    opus_head = b"OpusHead" + bytes([1, 1]) + struct.pack("<HIhB", 0, 48_000, 0, 0)
    opus_tags = b"OpusTags" + struct.pack("<I", 4) + b"fvid" + struct.pack("<I", 0)
    raw20 = bytes.fromhex("f8fffe")
    ogg_opus = b"".join(
        [
            ogg_page(opus_head, 0, 2, 0, 0x46560213),
            ogg_page(opus_tags, 0, 0, 1, 0x46560213),
            ogg_page(raw20, 960, 0, 2, 0x46560213),
            ogg_page(raw20, 1920, 4, 3, 0x46560213),
        ]
    )

    return {
        "video.mkv": owned_eof()["video.mkv"],
        "g711-alaw-48k-450ms.wav": g711_alaw,
        "g711-mulaw-48k-450ms.wav": g711_mulaw,
        "ogg-opus-mono-48k.ogg": ogg_opus,
        "raw-flac-48k-440ms.flac": build_raw_flac_48k(),
        "multitrack-pcm-flac.mka": build_multitrack_pcm_flac_mka(),
        "multitrack-pcm-ima4.mov": build_multitrack_pcm_ima4_mov(),
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
        "batch_id": "FVID-0213",
        "sha256": hashes,
    }
    (out_dir / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8")
    return hashes


if __name__ == "__main__":
    write_fixtures()
