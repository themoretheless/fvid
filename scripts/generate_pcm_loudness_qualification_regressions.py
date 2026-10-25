#!/usr/bin/env python3
"""Generate synthetic fixtures for FVID-0267 PCM loudness and normalization qualification, telemetry, and Matroska packet budgets."""

from __future__ import annotations

import hashlib
import json
from pathlib import Path
import struct


ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / "tests/fixtures/playback-errors/pcm-loudness-qualification"


def crc8(data: bytes) -> int:
    crc = 0
    for byte in data:
        crc ^= byte
        for _ in range(8):
            if crc & 0x80:
                crc = ((crc << 1) ^ 0x07) & 0xFF
            else:
                crc = (crc << 1) & 0xFF
    return crc


def crc16_flac(data: bytes) -> int:
    crc = 0
    for byte in data:
        crc ^= byte << 8
        for _ in range(8):
            if crc & 0x8000:
                crc = ((crc << 1) ^ 0x8005) & 0xFFFF
            else:
                crc = (crc << 1) & 0xFFFF
    return crc


def make_streaminfo(
    *,
    min_block: int,
    max_block: int,
    min_frame: int,
    max_frame: int,
    sample_rate: int,
    channels: int,
    bits_per_sample: int,
    total_samples: int,
) -> bytes:
    packed = (
        ((sample_rate & 0xFFFFF) << 44)
        + (((channels - 1) & 0x7) << 41)
        + (((bits_per_sample - 1) & 0x1F) << 36)
        + (total_samples & ((1 << 36) - 1))
    )
    return (
        struct.pack(">HH", min_block, max_block)
        + min_frame.to_bytes(3, "big")
        + max_frame.to_bytes(3, "big")
        + struct.pack(">Q", packed)
        + b"\x00" * 16
    )


def make_flac_frame(
    frame_number: int,
    *,
    channels: int,
    custom_rate_khz: int | None = None,
    subframe_values: list[int],
) -> bytes:
    assert len(subframe_values) == channels
    # block=4 -> 2304 samples (576 << 2)
    # rate=12 (0xC) if custom_rate_khz is given, else rate=10 (0xA = 48000 Hz)
    rate_code = 0x0C if custom_rate_khz is not None else 0x0A
    byte2 = (4 << 4) | rate_code
    # channel assignment = channels - 1, sample size code = 4 (16-bit), reserved=0 -> ((channels - 1) << 4) | 0x08
    byte3 = ((channels - 1) << 4) | 0x08
    header_prefix = bytearray([0xFF, 0xF8, byte2, byte3, frame_number])
    if custom_rate_khz is not None:
        header_prefix.append(custom_rate_khz)
    header = bytes(header_prefix) + bytes([crc8(header_prefix)])
    subframes = b"".join(b"\x00" + struct.pack(">h", v) for v in subframe_values)
    body = header + subframes
    return body + struct.pack(">H", crc16_flac(body))


def make_flac_file(
    *,
    sample_rate: int,
    channels: int,
    custom_rate_khz: int | None,
    frame_count: int,
    subframe_values: list[int],
    unassigned_mask: bool = False,
) -> bytes:
    frames = [
        make_flac_frame(
            idx,
            channels=channels,
            custom_rate_khz=custom_rate_khz,
            subframe_values=subframe_values,
        )
        for idx in range(frame_count)
    ]
    si = make_streaminfo(
        min_block=2304,
        max_block=2304,
        min_frame=min(len(f) for f in frames),
        max_frame=max(len(f) for f in frames),
        sample_rate=sample_rate,
        channels=channels,
        bits_per_sample=16,
        total_samples=2304 * frame_count,
    )
    if not unassigned_mask:
        return b"fLaC" + bytes([0x80, 0x00, 0x00, 34]) + si + b"".join(frames)
    vendor = b"fvid"
    entry = b"WAVEFORMATEXTENSIBLE_CHANNEL_MASK=0x0"
    vc_body = (
        struct.pack("<I", len(vendor))
        + vendor
        + struct.pack("<I", 1)
        + struct.pack("<I", len(entry))
        + entry
    )
    vc_header = bytes([0x84]) + len(vc_body).to_bytes(3, "big")
    return (
        b"fLaC"
        + bytes([0x00, 0x00, 0x00, 34])
        + si
        + vc_header
        + vc_body
        + b"".join(frames)
    )


def ebml_vint_len(length: int) -> bytes:
    if length < 0x7F:
        return bytes([0x80 | length])
    if length < 0x3FFF:
        return struct.pack(">H", 0x4000 | length)
    return struct.pack(">I", 0x10000000 | length)


def ebml_el(el_id: bytes, payload: bytes) -> bytes:
    return el_id + ebml_vint_len(len(payload)) + payload


def ebml_uint(el_id: bytes, val: int) -> bytes:
    if val == 0:
        b = b"\x00"
    else:
        n = (val.bit_length() + 7) // 8
        b = val.to_bytes(n, "big")
    return ebml_el(el_id, b)


def ebml_float(el_id: bytes, val: float) -> bytes:
    return ebml_el(el_id, struct.pack(">d", val))


def ebml_str(el_id: bytes, s: str) -> bytes:
    return ebml_el(el_id, s.encode("ascii"))


def make_video_audio_pcm_mkv() -> bytes:
    """Synthetic Matroska with Track 1 = V_UNCOMPRESSED (512-byte frame) and Track 2 = A_PCM/INT/LIT 48 kHz mono 16-bit (20 packets of 100 bytes each)."""
    ebml_head = ebml_el(
        b"\x1a\x45\xdf\xa3",
        ebml_uint(b"\x42\x86", 1)
        + ebml_uint(b"\x42\xf7", 1)
        + ebml_uint(b"\x42\xf2", 4)
        + ebml_uint(b"\x42\xf3", 8)
        + ebml_str(b"\x42\x82", "matroska")
        + ebml_uint(b"\x42\x87", 4)
        + ebml_uint(b"\x42\x85", 2),
    )
    info = ebml_el(b"\x15\x49\xa9\x66", ebml_uint(b"\x2a\xd7\xb1", 1_000_000))
    video_track = ebml_el(
        b"\xae",
        ebml_uint(b"\xd7", 1)
        + ebml_uint(b"\x73\xc5", 1)
        + ebml_uint(b"\x83", 1)
        + ebml_str(b"\x86", "V_UNCOMPRESSED")
        + ebml_el(b"\xe0", ebml_uint(b"\xb0", 16) + ebml_uint(b"\xba", 16)),
    )
    audio_track = ebml_el(
        b"\xae",
        ebml_uint(b"\xd7", 2)
        + ebml_uint(b"\x73\xc5", 2)
        + ebml_uint(b"\x83", 2)
        + ebml_str(b"\x86", "A_PCM/INT/LIT")
        + ebml_el(
            b"\xe1",
            ebml_float(b"\xb5", 48000.0)
            + ebml_uint(b"\x9f", 1)
            + ebml_uint(b"\x62\x64", 16),
        ),
    )
    tracks = ebml_el(b"\x16\x54\xae\x6b", video_track + audio_track)
    # Video SimpleBlock on track 1 (512-byte payload)
    v_sb = ebml_el(b"\xa3", b"\x81\x00\x00\x80" + b"\x10" * 512)
    # 20 Audio SimpleBlocks on track 2, each 960 samples = 1920 bytes (Wait: let's make each audio block 1920 bytes = 20 ms, so 25 blocks = 500 ms > 400 ms gate, and video block 4096 bytes!)
    v_sb = ebml_el(b"\xa3", b"\x81\x00\x00\x80" + b"\x10" * 4096)
    audio_pcm_block = struct.pack("<h", 4096) * 960  # 1920 bytes (960 samples = 20 ms at 48 kHz)
    a_sbs = b"".join(
        ebml_el(b"\xa3", b"\x82" + struct.pack(">h", idx * 20) + b"\x80" + audio_pcm_block)
        for idx in range(25)
    )
    cluster = ebml_el(b"\x1f\x43\xb6\x75", ebml_uint(b"\xe7", 0) + v_sb + a_sbs)
    segment = ebml_el(b"\x18\x53\x80\x67", info + tracks + cluster)
    return ebml_head + segment


def generate() -> dict[str, bytes]:
    # 1. 3-channel bare FLAC at 48 kHz (explicit unassigned speaker mask=0 in VORBIS_COMMENT, 10 frames * 2304 = 23040 samples = 480 ms > 400 ms gate)
    flac_3ch = make_flac_file(
        sample_rate=48_000,
        channels=3,
        custom_rate_khz=None,
        frame_count=10,
        subframe_values=[4096, 4096, 4096],
        unassigned_mask=True,
    )
    # 2. 1-channel bare FLAC at 20 kHz (non-standard sample rate valid for LoudnessMeter, 5 frames * 2304 = 11520 samples = 576 ms > 400 ms gate)
    flac_20khz = make_flac_file(
        sample_rate=20_000,
        channels=1,
        custom_rate_khz=20,
        frame_count=5,
        subframe_values=[4096],
    )
    # 3. 2-track Matroska with 4096-byte video packet on stream 0 and 1920-byte PCM audio packets on stream 1
    mkv_video_audio = make_video_audio_pcm_mkv()

    files = {
        "discrete-3ch-48k.flac": flac_3ch,
        "mono-20k.flac": flac_20khz,
        "video-4096-audio-1920.mkv": mkv_video_audio,
    }
    manifest = {
        "generator": "scripts/generate_pcm_loudness_qualification_regressions.py",
        "batch_id": "FVID-0267",
        "provenance": "100% synthetic bare FLAC (3ch 48 kHz discrete and 1ch 20 kHz non-standard rate) and two-track Matroska (4096-byte video + 1920-byte PCM audio); no FFmpeg, network, or private media.",
        "sha256": {name: hashlib.sha256(data).hexdigest() for name, data in files.items()},
    }
    files["manifest.json"] = (json.dumps(manifest, indent=2, sort_keys=True) + "\n").encode("utf-8")
    return files


def write_fixtures(out_dir: Path = OUT) -> dict[str, bytes]:
    out_dir.mkdir(parents=True, exist_ok=True)
    files = generate()
    for name, content in files.items():
        (out_dir / name).write_bytes(content)
    return files


if __name__ == "__main__":
    write_fixtures()
