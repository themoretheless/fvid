#!/usr/bin/env python3
"""Generate synthetic WAVE, G.711 WAVE, bare FLAC, Ogg Opus, bare MP3, and Matroska FLAC fixtures for sample-domain audio concat (.wav/.mka/.mkv) and metadata regressions (FVID-0232)."""

from __future__ import annotations

import hashlib
import json
import math
from pathlib import Path
import struct

from generate_flac_coded_numbers_regressions import frame as flac_frame, stream as flac_stream
from generate_loudnorm_budget_mix_concat_regressions import build_matroska_flac_mka
from generate_mp3_id3_metadata_regressions import build_id3v24_tag, id3v24_text_frame
from generate_wave_ogg_flac_plan_regressions import make_g711_wave, ogg_page


ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / "tests/fixtures/playback-errors/audio-sample-concat-wave-metadata"


def make_info_subchunk(fourcc: bytes, value: str) -> bytes:
    assert len(fourcc) == 4
    data = value.encode("utf-8") + b"\x00"
    pad = b"\x00" if len(data) % 2 else b""
    return fourcc + struct.pack("<I", len(data)) + data + pad


def build_tagged_pcm_wave(
    *,
    sample_rate: int = 48_000,
    channels: int = 1,
    frames: int = 1920,
    bits: int = 16,
    freq_hz: float = 440.0,
    tags: list[tuple[bytes, str]] | None = None,
) -> bytes:
    assert bits in (16, 24)
    byte_width = bits // 8
    block_align = channels * byte_width
    byte_rate = sample_rate * block_align
    fmt = struct.pack(
        "<HHIIHH",
        1,
        channels,
        sample_rate,
        byte_rate,
        block_align,
        bits,
    )
    pcm = bytearray()
    for i in range(frames):
        val = math.sin(2.0 * math.pi * freq_hz * i / sample_rate)
        for _ in range(channels):
            if bits == 16:
                pcm += struct.pack("<h", int(round(val * 12000.0)))
            else:
                sample = int(round(val * 3000000.0))
                pcm += sample.to_bytes(4, "little", signed=True)[:3]
    info_body = b"INFO" + b"".join(
        make_info_subchunk(fourcc, text) for fourcc, text in (tags or [])
    )
    list_chunk = (
        (b"LIST" + struct.pack("<I", len(info_body)) + info_body)
        if tags
        else b""
    )
    pad = b"\x00" if len(pcm) % 2 else b""
    body = (
        b"WAVE"
        + b"fmt "
        + struct.pack("<I", len(fmt))
        + fmt
        + b"data"
        + struct.pack("<I", len(pcm))
        + bytes(pcm)
        + pad
        + list_chunk
    )
    return b"RIFF" + struct.pack("<I", len(body)) + body


def build_raw_flac_48k() -> bytes:
    packets = [
        flac_frame(i, 1920, False, 4096 if i % 2 == 0 else -4096, block_code=7, rate_code=10)
        for i in range(2)
    ]
    return flac_stream(packets, [1920] * 2, rate=48_000)


def build_ogg_opus_48k() -> bytes:
    opus_head = b"OpusHead" + bytes([1, 1]) + struct.pack("<HIhB", 0, 48_000, 0, 0)
    opus_tags = b"OpusTags" + struct.pack("<I", 4) + b"fvid" + struct.pack("<I", 0)
    raw20 = bytes.fromhex("f8fffe")
    return b"".join(
        [
            ogg_page(opus_head, 0, 2, 0, 0x46560232),
            ogg_page(opus_tags, 0, 0, 1, 0x46560232),
            ogg_page(raw20, 960, 0, 2, 0x46560232),
            ogg_page(raw20, 1920, 4, 3, 0x46560232),
        ]
    )


def build_tagged_mp3_48k() -> bytes:
    raw_mp3 = (
        ROOT / "tests/fixtures/playback-errors/mp3-decoder/48000-1ch.mp3"
    ).read_bytes()
    tag = build_id3v24_tag(
        [
            id3v24_text_frame("TIT2", "Concat MP3 Source"),
            id3v24_text_frame("TPE1", "FVid Concat Artist"),
        ]
    )
    return tag + raw_mp3


def generate() -> dict[str, bytes]:
    codes_alaw = bytes((0x80 if (i % 16 < 8) else 0x00) for i in range(1920))
    g711_alaw = make_g711_wave(law="alaw", sample_rate=48_000, channels=1, codes=codes_alaw)

    pcm16_tagged = build_tagged_pcm_wave(
        sample_rate=48_000,
        channels=1,
        frames=1920,
        bits=16,
        freq_hz=440.0,
        tags=[
            (b"INAM", "Original Wave Segment A"),
            (b"IART", "Original Wave Artist"),
            (b"ICMT", "Drop Wave Comment"),
        ],
    )
    pcm24_b = build_tagged_pcm_wave(
        sample_rate=48_000,
        channels=1,
        frames=1920,
        bits=24,
        freq_hz=660.0,
        tags=[
            (b"INAM", "Second Wave Segment B"),
        ],
    )

    files = {
        "pcm16-tagged-48k.wav": pcm16_tagged,
        "pcm24-48k.wav": pcm24_b,
        "g711-alaw-48k.wav": g711_alaw,
        "raw-flac-48k.flac": build_raw_flac_48k(),
        "ogg-opus-48k.ogg": build_ogg_opus_48k(),
        "tagged-48k.mp3": build_tagged_mp3_48k(),
        "matroska-flac-48k.mka": build_matroska_flac_mka(),
    }
    manifest = {
        "batch_id": "FVID-0232",
        "sha256": {name: hashlib.sha256(data).hexdigest() for name, data in files.items()},
    }
    files["manifest.json"] = (json.dumps(manifest, indent=2, sort_keys=True) + "\n").encode(
        "utf-8"
    )
    return files


def main() -> None:
    OUT.mkdir(parents=True, exist_ok=True)
    for name, data in generate().items():
        (OUT / name).write_bytes(data)


if __name__ == "__main__":
    main()
