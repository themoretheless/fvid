#!/usr/bin/env python3
"""Generate synthetic G.711 WAVE, Ogg Opus/Vorbis/Joint, bare FLAC, and bare MP3 probe/loudness/loudnorm regressions for FVID-0209."""

from __future__ import annotations

import hashlib
import json
from pathlib import Path
import struct

from generate_audio_reader_eof_regressions import generate as owned_eof
from generate_wave_ogg_flac_plan_regressions import (
    make_g711_wave,
    make_opus_link,
    make_vorbis_link,
    ogg_page,
)


ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / "tests/fixtures/playback-errors/probe-loudness-audio"


def mp3_crc16(data: bytes) -> int:
    crc = 0
    for byte in data:
        crc ^= byte
        for _ in range(8):
            crc = (crc >> 1) ^ (0xA001 if (crc & 1) else 0)
    return crc


def make_lame_info_frame(
    *,
    header: bytes,
    frame_size: int,
    side_bytes: int,
    coded_frames: int,
    encoder_delay: int,
    end_padding: int,
) -> bytes:
    """Build a valid MPEG-1 Layer III Info frame with a CRC-verified revision-0 LAME tag."""
    assert len(header) == 4
    buf = bytearray(frame_size)
    buf[:4] = header
    offset = 4 + side_bytes
    buf[offset : offset + 4] = b"Info"
    struct.pack_into(">I", buf, offset + 4, 1)  # flags = 1 (frames field present)
    struct.pack_into(">I", buf, offset + 8, coded_frames)
    at = offset + 12
    # 36-byte LAME revision-0 tag
    tag = bytearray(36)
    tag[:9] = b"LAME3.100"
    tag[9] = 0x00  # revision 0 (high nibble 0)
    tag[21] = (encoder_delay >> 4) & 0xFF
    tag[22] = ((encoder_delay & 0x0F) << 4) | ((end_padding >> 8) & 0x0F)
    tag[23] = end_padding & 0xFF
    buf[at : at + 34] = tag[:34]
    crc = mp3_crc16(bytes(buf[: at + 34]))
    struct.pack_into(">H", buf, at + 34, crc)
    return bytes(buf)


def generate() -> dict[str, bytes]:
    # 1. G.711 A-law and mu-law WAVE at 48 kHz mono with title metadata and non-silent signal
    codes_alaw = bytes((0x55 if (i % 16 < 8) else 0xD5) for i in range(4800))
    codes_mulaw = bytes((0x1F if (i % 16 < 8) else 0x9F) for i in range(4800))
    g711_alaw = make_g711_wave(law="alaw", sample_rate=48_000, channels=1, codes=codes_alaw)
    g711_mulaw = make_g711_wave(law="mulaw", sample_rate=48_000, channels=1, codes=codes_mulaw)

    # 2. Ogg Opus, Ogg Vorbis, and Ogg Joint (Vorbis -> Opus) at 48 kHz stereo
    # Add comment metadata (TITLE=Synthetic Opus, LANGUAGE=eng) on the Opus link
    opus_head = b"OpusHead" + bytes([1, 2]) + struct.pack("<HIhB", 0, 48_000, 0, 0)
    opus_comments = [b"TITLE=Synthetic Opus", b"LANGUAGE=eng"]
    opus_tags = (
        b"OpusTags"
        + struct.pack("<I", 4)
        + b"fvid"
        + struct.pack("<I", len(opus_comments))
        + b"".join(struct.pack("<I", len(c)) + c for c in opus_comments)
    )
    raw20 = bytes.fromhex("f8fffe")
    ogg_opus = b"".join(
        [
            ogg_page(opus_head, 0, 2, 0, 0x46560209),
            ogg_page(opus_tags, 0, 0, 1, 0x46560209),
            ogg_page(raw20, 960, 0, 2, 0x46560209),
            ogg_page(raw20, 1920, 4, 3, 0x46560209),
        ]
    )
    ogg_vorbis = make_vorbis_link(serial=0x4656020A, channels=2, sample_rate=48_000)
    ogg_joint = (
        make_vorbis_link(serial=0x4656020B, channels=2, sample_rate=48_000)
        + make_opus_link(serial=0x4656020C, channels=2)
    )

    # 3. Bare FLAC (4-channel quad 0x33) and bare MP3 (48 kHz stereo)
    raw_flac = (ROOT / "tests/fixtures/playback-errors/flac-native/extended-lpc-4-1-0-0.flac").read_bytes()
    raw_mp3 = (ROOT / "tests/fixtures/playback-errors/mpeg-clock-metadata/mp3-48000.mp3").read_bytes()

    # 4. Bare MP3 whose LAME Info tag trims all coded samples (4 frames = 4608 coded samples;
    #    encoder_delay=2000, end_padding=2608 -> start=2529, end=2529 -> presentation_samples() == 0).
    #    Use an unprotected (CRC-off, bit 0 of byte 1 set) MPEG-1 Layer III 48 kHz stereo 320 kbps
    #    Info header (frame_size=960, side_bytes=32).
    mp3_header = bytes([raw_mp3[0], raw_mp3[1] | 0x01, raw_mp3[2], raw_mp3[3]])
    info_frame = make_lame_info_frame(
        header=mp3_header,
        frame_size=960,
        side_bytes=32,
        coded_frames=4,
        encoder_delay=2000,
        end_padding=2608,
    )
    raw_mp3_empty_trimmed = info_frame + raw_mp3

    files = {
        "video.mkv": owned_eof()["video.mkv"],
        "g711-alaw-48k.wav": g711_alaw,
        "g711-mulaw-48k.wav": g711_mulaw,
        "ogg-opus-48k.ogg": ogg_opus,
        "ogg-vorbis-48k.ogg": ogg_vorbis,
        "ogg-joint-48k.ogg": ogg_joint,
        "raw-flac-48k.flac": raw_flac,
        "raw-mp3-48k.mp3": raw_mp3,
        "raw-mp3-empty-trimmed.mp3": raw_mp3_empty_trimmed,
    }
    manifest = {
        "generator": "scripts/generate_probe_loudness_audio_regressions.py",
        "batch_id": "FVID-0209",
        "provenance": "100% synthetic G.711 WAVE, Ogg Opus/Vorbis/Joint, bare FLAC, and bare MP3 fixtures; no FFmpeg, network, or private media.",
        "sha256": {name: hashlib.sha256(data).hexdigest() for name, data in files.items()},
    }
    files["manifest.json"] = (json.dumps(manifest, indent=2) + "\n").encode("utf-8")
    return files


def main() -> None:
    OUT.mkdir(parents=True, exist_ok=True)
    for name, data in generate().items():
        (OUT / name).write_bytes(data)


if __name__ == "__main__":
    main()
