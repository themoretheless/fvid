#!/usr/bin/env python3
"""Generate deterministic synthetic fixtures for Batch 238 (`FVID-0245`).

Covers:
1. 8-channel 7.1 extensible WAVE (`wave-8ch-71.wav`, mask `0x63f`) and stereo
   WAVE (`wave-stereo.wav`, mask `0x3`) for native audio rematrixing (`7.1 -> 1 | 2`
   and `1 | 2 -> 3..=8`) and `plan_decode_audio` / `decode_audio_transformed`.
2. 6-channel 5.1 Matroska Opus (`opus-6ch-51.mka`, mapping family 1) for
   `owned_matroska_opus_export::supports` channel layout qualification even when
   packet/budget limits fail.
3. Stereo Matroska MP2 (`mp2-stereo.mka`) and bare MPEG Layer II (`mp2-stereo-tagged.mp2`)
   for `owned_matroska_mp3_export::supports` channel conversion bounds and
   `owned_raw_mp3::source_info` with WAVE metadata edits.
"""

from __future__ import annotations

import hashlib
import json
import math
from pathlib import Path
import struct

from generate_audio_resample_window_fixture import ebml
from generate_mp3_id3_metadata_regressions import (
    build_id3v24_tag,
    id3v24_text_frame,
)


ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / "tests/fixtures/playback-errors/audio-rematrix-opus-mp3-admission"


def u(tag: str, n: int) -> bytes:
    return ebml(tag, n.to_bytes(max(1, (n.bit_length() + 7) // 8), "big"))


def f64_elem(tag: str, val: float) -> bytes:
    return ebml(tag, struct.pack(">d", val))


def simple_block(track_no: int, rel_ms: int, payload: bytes) -> bytes:
    assert 1 <= track_no <= 126
    return ebml("a3", bytes([0x80 | track_no]) + struct.pack(">hB", rel_ms, 0x80) + payload)


def build_extensible_float_wave(
    sample_rate: int,
    channels: int,
    channel_mask: int,
    frames: int,
) -> bytes:
    pcm = bytearray()
    for frame in range(frames):
        for ch in range(channels):
            # Distinct deterministic amplitude per speaker index so speaker fold is verifiable
            val = 0.05 * float(ch + 1) * math.cos(2.0 * math.pi * (frame / 16.0))
            pcm.extend(struct.pack("<f", val))

    block_align = channels * 4
    byte_rate = sample_rate * block_align
    subformat_ieee_float = bytes(
        [
            3, 0, 0, 0,
            0, 0,
            0x10, 0,
            0x80, 0,
            0, 0xAA, 0, 0x38, 0x9B, 0x71,
        ]
    )
    fmt_body = (
        struct.pack(
            "<HHIIHHHH I",
            0xFFFE,
            channels,
            sample_rate,
            byte_rate,
            block_align,
            32,
            22,
            32,
            channel_mask,
        )
        + subformat_ieee_float
    )
    fmt_chunk = b"fmt " + struct.pack("<I", len(fmt_body)) + fmt_body
    fact_chunk = b"fact" + struct.pack("<II", 4, frames)
    data_chunk = b"data" + struct.pack("<I", len(pcm)) + bytes(pcm)
    riff_body = b"WAVE" + fmt_chunk + fact_chunk + data_chunk
    return b"RIFF" + struct.pack("<I", len(riff_body)) + riff_body


def build_opus_6ch_51_mka() -> bytes:
    header = ebml("1a45dfa3", ebml("4282", b"matroska") + u("4287", 4) + u("4285", 2))
    info = ebml(
        "1549a966",
        u("2ad7b1", 1_000_000) + f64_elem("4489", 40.0) + ebml("5741", b"FVidOpus51Synth"),
    )
    # OpusHead for 6-channel 5.1 (mapping family 1, stream_count=4, coupled_count=2, mapping=[0,4,1,2,3,5])
    opus_head = (
        b"OpusHead"
        + bytes([1, 6])
        + struct.pack("<HIhB", 0, 48000, 0, 1)
        + bytes([4, 2, 0, 4, 1, 2, 3, 5])
    )
    track = ebml(
        "ae",
        u("d7", 1)
        + u("73c5", 5101)
        + u("83", 2)
        + ebml("86", b"A_OPUS")
        + ebml("63a2", opus_head)
        + u("56aa", 0)
        + u("23e383", 20_000_000)
        + ebml("536e", b"SurroundOpus")
        + ebml("22b59c", b"eng")
        + ebml("e1", f64_elem("b5", 48000.0) + u("9f", 6)),
    )
    tracks = ebml("1654ae6b", track)
    # 4 internal streams in mapping family 1: 3 self-delimited + 1 final
    # Self-delimited CELT 20ms silence packet: [0xf8, 0x02, 0xff, 0xfe] (TOC=0xf8, len=2, payload=0xfffe)
    # Final stream packet: [0xf8, 0xff, 0xfe]
    pkt = bytes([0xF8, 0x02, 0xFF, 0xFE]) * 3 + bytes([0xF8, 0xFF, 0xFE])
    cluster = ebml(
        "1f43b675",
        u("e7", 0) + simple_block(1, 0, pkt) + simple_block(1, 20, pkt),
    )
    return header + ebml("18538067", info + tracks + cluster)


def build_synthetic_mp2_frames() -> tuple[list[bytes], bytes]:
    """Build two deterministic MPEG-1 Layer II 48 kHz stereo 64 kbps frames (192 bytes each)."""
    # MPEG-1 (version=3 -> bits 11), Layer II (layer=2 -> bits 10), no CRC (1),
    # bitrate_index=4 (64 kbps in Layer II), samplerate_index=1 (48000 Hz), padding=0, mode=0 (stereo):
    # 0xFF, 0xFD, (4 << 4) | (1 << 2) = 0x44, 0x00
    # Frame size = 144 * 64000 // 48000 = 192 bytes.
    frame = bytes([0xFF, 0xFD, 0x44, 0x00]) + bytes(188)
    frames = [frame, frame]
    tag = build_id3v24_tag(
        [
            id3v24_text_frame("TIT2", "Tagged Bare MP2"),
            id3v24_text_frame("TPE1", "FVid Synth Artist"),
            id3v24_text_frame("TLAN", "fra"),
        ]
    )
    return frames, tag + b"".join(frames)


def build_mp2_stereo_mka(frames: list[bytes]) -> bytes:
    header = ebml("1a45dfa3", ebml("4282", b"matroska") + u("4287", 4) + u("4285", 2))
    info = ebml(
        "1549a966",
        u("2ad7b1", 1_000_000) + f64_elem("4489", 48.0) + ebml("5741", b"FVidMp2MkaSynth"),
    )
    track = ebml(
        "ae",
        u("d7", 1)
        + u("73c5", 5201)
        + u("83", 2)
        + ebml("86", b"A_MPEG/L2")
        + u("23e383", 24_000_000)
        + ebml("536e", b"StereoMp2")
        + ebml("22b59c", b"eng")
        + ebml("e1", f64_elem("b5", 48000.0) + u("9f", 2)),
    )
    tracks = ebml("1654ae6b", track)
    cluster = ebml(
        "1f43b675",
        u("e7", 0)
        + simple_block(1, 0, frames[0])
        + simple_block(1, 24, frames[1]),
    )
    return header + ebml("18538067", info + tracks + cluster)


def generate() -> dict[str, bytes]:
    mp2_frames, mp2_tagged = build_synthetic_mp2_frames()
    files = {
        "wave-8ch-71.wav": build_extensible_float_wave(48000, 8, 0x63F, 64),
        "wave-stereo.wav": build_extensible_float_wave(48000, 2, 0x3, 64),
        "opus-6ch-51.mka": build_opus_6ch_51_mka(),
        "mp2-stereo.mka": build_mp2_stereo_mka(mp2_frames),
        "mp2-stereo-tagged.mp2": mp2_tagged,
    }
    manifest = {
        "batch_id": "FVID-0245",
        "generator": "scripts/generate_audio_rematrix_opus_mp3_admission_regressions.py",
        "files": {
            name: {
                "bytes": len(data),
                "sha256": hashlib.sha256(data).hexdigest(),
            }
            for name, data in sorted(files.items())
        },
    }
    files["manifest.json"] = (json.dumps(manifest, indent=2, sort_keys=True) + "\n").encode("utf-8")
    return files


def main() -> None:
    OUT.mkdir(parents=True, exist_ok=True)
    for name, data in generate().items():
        (OUT / name).write_bytes(data)


if __name__ == "__main__":
    main()
