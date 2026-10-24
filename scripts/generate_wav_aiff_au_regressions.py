#!/usr/bin/env python3
"""Generate synthetic WAV, AIFF/AIFC, and AU regression fixtures for FVID-0193."""

from __future__ import annotations

import json
from pathlib import Path
import struct


ROOT = Path(__file__).resolve().parents[1]
FIXTURES_DIR = ROOT / "tests" / "fixtures" / "playback-errors" / "wav-aiff-au"

KSDATAFORMAT_SUBTYPE_TAIL = bytes(
    [0x00, 0x00, 0x10, 0x00, 0x80, 0x00, 0x00, 0xAA, 0x00, 0x38, 0x9B, 0x71]
)
RATE_48000_EXT80 = bytes(
    [0x40, 0x0E, 0xBB, 0x80, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00]
)


def make_riff_wave(fmt_body: bytes, data_body: bytes) -> bytes:
    chunks = bytearray()
    for tag, body in ((b"fmt ", fmt_body), (b"data", data_body)):
        chunks += tag
        chunks += struct.pack("<I", len(body))
        chunks += body
        if len(body) % 2 == 1:
            chunks += b"\x00"
    return b"RIFF" + struct.pack("<I", 4 + len(chunks)) + b"WAVE" + bytes(chunks)


def make_extensible_fmt(
    *,
    channels: int,
    sample_rate: int,
    bits_per_sample: int,
    cb_size: int,
    valid_bits: int,
    channel_mask: int,
    subformat_tag: int,
) -> bytes:
    block_align = channels * (bits_per_sample // 8)
    byte_rate = sample_rate * block_align
    return (
        struct.pack(
            "<HHIIHHHHI",
            0xFFFE,
            channels,
            sample_rate,
            byte_rate,
            block_align,
            bits_per_sample,
            cb_size,
            valid_bits,
            channel_mask,
        )
        + struct.pack("<I", subformat_tag)
        + KSDATAFORMAT_SUBTYPE_TAIL
    )


def make_form_aifc(
    *,
    channels: int,
    frames: int,
    bits_per_sample: int,
    compression: bytes,
    samples: bytes,
) -> bytes:
    assert len(compression) == 4
    fver_body = struct.pack(">I", 0xA2805140)
    comm_body = (
        struct.pack(">hIh", channels, frames, bits_per_sample)
        + RATE_48000_EXT80
        + compression
        + b"\x00\x00"
    )
    ssnd_body = struct.pack(">II", 0, 0) + samples
    chunks = bytearray()
    for tag, body in ((b"FVER", fver_body), (b"COMM", comm_body), (b"SSND", ssnd_body)):
        chunks += tag
        chunks += struct.pack(">I", len(body))
        chunks += body
        if len(body) % 2 == 1:
            chunks += b"\x00"
    return b"FORM" + struct.pack(">I", 4 + len(chunks)) + b"AIFC" + bytes(chunks)


def make_au_pcm16(frames: list[int], sample_rate: int = 48_000, channels: int = 1) -> bytes:
    payload = b"".join(struct.pack(">h", s) for s in frames)
    header = b".snd" + struct.pack(">IIIII", 24, len(payload), 3, sample_rate, channels)
    return header + payload


def build_fixtures() -> dict[str, bytes]:
    pcm16_mono_samples = struct.pack("<hhhh", 0, 2048, -2048, 4096)
    g711_alaw_samples = bytes([0xD5, 0x55, 0xAA, 0x2A])

    return {
        "wav_extensible_channel_mask_overflow.wav": make_riff_wave(
            make_extensible_fmt(
                channels=1,
                sample_rate=48_000,
                bits_per_sample=16,
                cb_size=22,
                valid_bits=16,
                channel_mask=0b11,  # 2 speaker positions for 1 channel
                subformat_tag=1,
            ),
            pcm16_mono_samples,
        ),
        "wav_extensible_cbsize_overflow.wav": make_riff_wave(
            make_extensible_fmt(
                channels=1,
                sample_rate=48_000,
                bits_per_sample=16,
                cb_size=30,  # claims 30 extension bytes in a 40-byte fmt chunk (22 available)
                valid_bits=16,
                channel_mask=0x04,
                subformat_tag=1,
            ),
            pcm16_mono_samples,
        ),
        "wav_extensible_valid.wav": make_riff_wave(
            make_extensible_fmt(
                channels=1,
                sample_rate=48_000,
                bits_per_sample=16,
                cb_size=22,
                valid_bits=16,
                channel_mask=0x04,
                subformat_tag=1,
            ),
            pcm16_mono_samples,
        ),
        "aifc_alaw_16bit_invalid.aifc": make_form_aifc(
            channels=1,
            frames=4,
            bits_per_sample=16,
            compression=b"alaw",
            samples=g711_alaw_samples,
        ),
        "aifc_ulaw_24bit_invalid.aifc": make_form_aifc(
            channels=1,
            frames=4,
            bits_per_sample=24,
            compression=b"ulaw",
            samples=g711_alaw_samples,
        ),
        "aifc_alaw_8bit_valid.aifc": make_form_aifc(
            channels=1,
            frames=4,
            bits_per_sample=8,
            compression=b"alaw",
            samples=g711_alaw_samples,
        ),
        "au_pcm16_valid.au": make_au_pcm16([0, 1024, -1024, 2048, -2048, 4096]),
    }


def write_fixtures(out_dir: Path = FIXTURES_DIR) -> dict[str, str]:
    out_dir.mkdir(parents=True, exist_ok=True)
    fixtures = build_fixtures()
    manifest_entries = []
    for name, content in sorted(fixtures.items()):
        path = out_dir / name
        path.write_bytes(content)
        manifest_entries.append(
            {
                "file": name,
                "bytes": len(content),
            }
        )
    manifest = {
        "suite": "wav-aiff-au",
        "improvement_id": "FVID-0193",
        "fixtures": manifest_entries,
    }
    (out_dir / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")
    return {name: str(out_dir / name) for name in fixtures}


if __name__ == "__main__":
    write_fixtures()
