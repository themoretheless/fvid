#!/usr/bin/env python3
"""Generate synthetic WAVE, ADTS AAC, MP4 PCM, and Matroska FLAC/PCM fixtures for loudnorm, decode-audio, mix-audio, merge-audio, and concat packet-budget preflight parity regressions (FVID-0248)."""

from __future__ import annotations

import hashlib
import json
from pathlib import Path
import struct

from generate_audio_reader_eof_regressions import generate as owned_eof
from generate_audio_resample_window_fixture import ebml
from generate_loudnorm_budget_mix_concat_regressions import (
    build_adts_48k,
    build_matroska_flac_mka,
    build_mp4_pcm_m4a,
    simple_block,
    u,
)


ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / "tests/fixtures/playback-errors/loudnorm-mix-packet-budget-parity"


def build_wave_s16le_48k() -> bytes:
    """Synthetic 48 kHz stereo 16-bit PCM WAVE (1920 sample frames, 4 bytes per frame)."""
    frames = 1920
    pcm = b"".join(
        struct.pack("<hh", 4096 if (i // 32) % 2 == 0 else -4096, -4096 if (i // 32) % 2 == 0 else 4096)
        for i in range(frames)
    )
    fmt = struct.pack("<HHIIHH", 1, 2, 48_000, 48_000 * 4, 4, 16)
    riff_payload = b"WAVEfmt " + struct.pack("<I", len(fmt)) + fmt + b"data" + struct.pack("<I", len(pcm)) + pcm
    return b"RIFF" + struct.pack("<I", len(riff_payload)) + riff_payload


def build_matroska_pcm_mka() -> bytes:
    """Single-track Matroska A_PCM/INT/LIT (16-bit 48 kHz mono, 2 packets of 1920 samples = 3840 bytes each)."""
    pkt_samples = 1920
    pkt_payload = b"".join(
        struct.pack("<h", 4096 if (i // 32) % 2 == 0 else -4096) for i in range(pkt_samples)
    )
    header = ebml("1a45dfa3", ebml("4282", b"matroska") + u("4287", 4) + u("4285", 2))
    info = ebml("1549a966", u("2ad7b1", 1_000_000))
    audio_settings = ebml("e1", ebml("b5", struct.pack(">d", 48000.0)) + u("9f", 1) + u("6264", 16))
    track = ebml(
        "ae",
        u("d7", 1)
        + u("73c5", 248)
        + u("83", 2)
        + ebml("86", b"A_PCM/INT/LIT")
        + audio_settings,
    )
    tracks = ebml("1654ae6b", track)
    blocks = u("e7", 0)
    blocks += simple_block(1, 0, 0x80, pkt_payload)
    blocks += simple_block(1, 40, 0x80, pkt_payload)
    return header + ebml("18538067", info + tracks + ebml("1f43b675", blocks))


def generate() -> dict[str, bytes]:
    files = {
        "video.mkv": owned_eof()["video.mkv"],
        "stereo-48k.wav": build_wave_s16le_48k(),
        "adts-mono-48k.aac": build_adts_48k(),
        "mp4-pcm-48k.m4a": build_mp4_pcm_m4a(),
        "matroska-flac-48k.mka": build_matroska_flac_mka(),
        "matroska-pcm-48k.mka": build_matroska_pcm_mka(),
    }
    manifest = {
        "batch_id": "FVID-0248",
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
