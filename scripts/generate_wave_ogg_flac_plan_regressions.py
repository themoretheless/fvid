#!/usr/bin/env python3
"""Generate synthetic WAVE/Ogg/FLAC/MP3 audio planning, padding, and rematrix regressions for FVID-0208."""

from __future__ import annotations

import hashlib
import json
from pathlib import Path
import struct

from generate_audio_reader_eof_regressions import generate as owned_eof


ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / "tests/fixtures/playback-errors/wave-ogg-flac-plan"


def riff_chunk(tag: bytes, payload: bytes) -> bytes:
    return tag + struct.pack("<I", len(payload)) + payload + (b"\x00" if len(payload) & 1 else b"")


def make_wave_extensible_int(
    *,
    sample_rate: int,
    channels: int,
    bits_per_sample: int,
    valid_bits: int,
    channel_mask: int,
    pcm_bytes: bytes,
) -> bytes:
    block_align = channels * (bits_per_sample // 8)
    byte_rate = sample_rate * block_align
    assert len(pcm_bytes) % block_align == 0
    sample_frames = len(pcm_bytes) // block_align
    ksdataformat_subtype_pcm = bytes(
        [0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x10, 0x00, 0x80, 0x00, 0x00, 0xAA, 0x00, 0x38, 0x9B, 0x71]
    )
    fmt = (
        struct.pack(
            "<HHIIHHHHI",
            0xFFFE,
            channels,
            sample_rate,
            byte_rate,
            block_align,
            bits_per_sample,
            22,
            valid_bits,
            channel_mask,
        )
        + ksdataformat_subtype_pcm
    )
    assert len(fmt) == 40
    body = (
        b"WAVE"
        + riff_chunk(b"fmt ", fmt)
        + riff_chunk(b"fact", struct.pack("<I", sample_frames))
        + riff_chunk(b"data", pcm_bytes)
    )
    return b"RIFF" + struct.pack("<I", len(body)) + body


def make_g711_wave(*, law: str, sample_rate: int, channels: int, codes: bytes) -> bytes:
    tag = 6 if law == "alaw" else 7
    assert len(codes) % channels == 0
    frames = len(codes) // channels
    fmt = struct.pack("<HHIIHHH", tag, channels, sample_rate, sample_rate * channels, channels, 8, 0)
    info = riff_chunk(b"LIST", b"INFO" + riff_chunk(b"INAM", b"Synthetic G711\x00"))
    body = (
        b"WAVE"
        + riff_chunk(b"fmt ", fmt)
        + riff_chunk(b"fact", struct.pack("<I", frames))
        + info
        + riff_chunk(b"data", codes)
    )
    return b"RIFF" + struct.pack("<I", len(body)) + body


def ogg_sealed(payload: bytes, laces: list[int], granule: int, flags: int, sequence: int, serial: int) -> bytes:
    assert sum(laces) == len(payload)
    buf = bytearray(
        b"OggS"
        + bytes([0, flags])
        + struct.pack("<qII", granule, serial, sequence)
        + bytes(4)
        + bytes([len(laces)])
        + bytes(laces)
        + payload
    )
    crc = 0
    for b in buf:
        crc ^= b << 24
        for _ in range(8):
            crc = ((crc << 1) ^ 0x04C11DB7 if crc & 0x80000000 else crc << 1) & 0xFFFFFFFF
    struct.pack_into("<I", buf, 22, crc)
    return bytes(buf)


def ogg_page(payload: bytes, granule: int, flags: int, sequence: int, serial: int) -> bytes:
    laces = [255] * (len(payload) // 255) + [len(payload) % 255]
    return ogg_sealed(payload, laces, granule, flags, sequence, serial)


def pack_vorbis_bits(fields: list[tuple[int, int]]) -> bytes:
    out = bytearray()
    at = 0
    for value, width in fields:
        for i in range(width):
            if at % 8 == 0:
                out.append(0)
            out[at // 8] |= ((value >> i) & 1) << (at % 8)
            at += 1
    return bytes(out)


def make_vorbis_link(*, serial: int, channels: int, sample_rate: int = 48_000, eos: bool = True) -> bytes:
    ident = b"\x01vorbis" + struct.pack("<IBIiiiBB", 0, channels, sample_rate, 0, 0, 0, 0x86, 1)
    comment = b"\x03vorbis" + struct.pack("<I", 4) + b"fvid" + struct.pack("<I", 0) + b"\x01"
    fields = (
        [(5, 8)]
        + [(b, 8) for b in b"vorbis"]
        + [
            (0, 8),  # 1 codebook
            (0x564342, 24),
            (1, 16),
            (1, 24),
            (0, 1),
            (0, 1),
            (0, 5),
            (0, 4),
            (0, 6),  # 1 time domain transform
            (0, 16),
            (0, 6),  # 1 floor
            (1, 16),
            (0, 5),
            (0, 2),
            (6, 4),
            (0, 6),  # 1 residue
            (0, 16),
            (0, 24),
            (128, 24),
            (31, 24),
            (0, 6),
            (0, 8),
            (0, 3),
            (0, 1),
            (0, 6),  # 1 mapping
            (0, 16),
            (0, 1),  # 1 submap
            (0, 1),  # no coupling
            (0, 2),  # reserved
            (0, 8),  # submap time
            (0, 8),  # submap floor
            (0, 8),  # submap residue
            (2, 6),  # 3 modes
        ]
    )
    for long_flag in [0, 0, 1]:
        fields.extend([(long_flag, 1), (0, 16), (0, 16), (0, 8)])
    setup = pack_vorbis_bits(fields + [(1, 1)])
    modes = [0, 2, 1, 2, 2, 0]
    packets = []
    for i, mode in enumerate(modes):
        header = [(0, 1), (mode, 2)]
        if mode == 2:
            header.extend(
                [
                    (int(i > 0 and modes[i - 1] == 2), 1),
                    (int(i + 1 < len(modes) and modes[i + 1] == 2), 1),
                ]
            )
        # Floor 1 unused flag (0, 1) per channel.
        packets.append(pack_vorbis_bits(header + [(0, 1)] * channels))
    data = (
        ogg_page(ident, 0, 2, 0, serial)
        + ogg_page(comment, 0, 0, 1, serial)
        + ogg_page(setup, 0, 0, 2, serial)
    )
    ends = [0, 80, 160, 240, 368, 448]
    for i, pkt in enumerate(packets):
        granule = (411 if eos else 448) if i == 5 else ends[i] if i == 1 else -1
        flags = (4 if eos else 0) if i == 5 else 0
        data += ogg_page(pkt, granule, flags, i + 3, serial)
    return data


def make_opus_link(*, serial: int, channels: int = 2) -> bytes:
    head = b"OpusHead" + bytes([1, channels]) + struct.pack("<HIhB", 0, 48_000, 0, 0)
    vendor = b"fvid"
    tags = b"OpusTags" + struct.pack("<I", len(vendor)) + vendor + struct.pack("<I", 0)
    raw20 = bytes.fromhex("f8fffe")
    return b"".join(
        [
            ogg_page(head, 0, 2, 0, serial),
            ogg_page(tags, 0, 0, 1, serial),
            ogg_page(raw20, 960, 0, 2, serial),
            ogg_page(raw20, 1920, 4, 3, serial),
        ]
    )


def generate() -> dict[str, bytes]:
    # 1. WAVEFORMATEXTENSIBLE 24-in-32 integer PCM with valid zero low padding vs non-zero low padding.
    valid_samples_s32 = [0x1000_0000 if (i % 4 < 2) else -0x1000_0000 for i in range(480)]
    valid_pcm_bytes = b"".join(struct.pack("<i", s) for s in valid_samples_s32)
    wave_24in32_valid = make_wave_extensible_int(
        sample_rate=48_000,
        channels=1,
        bits_per_sample=32,
        valid_bits=24,
        channel_mask=0x4,
        pcm_bytes=valid_pcm_bytes,
    )
    bad_samples_s32 = list(valid_samples_s32)
    bad_samples_s32[7] |= 0x01  # Non-zero low padding bit in the 8 unused LSBs!
    bad_pcm_bytes = b"".join(struct.pack("<i", s) for s in bad_samples_s32)
    wave_24in32_nonzero_padding = make_wave_extensible_int(
        sample_rate=48_000,
        channels=1,
        bits_per_sample=32,
        valid_bits=24,
        channel_mask=0x4,
        pcm_bytes=bad_pcm_bytes,
    )

    # 2. G.711 A-law and mu-law WAVE inputs at 48 kHz (mono) for plan_decode_audio / plan_mix_audio / plan_merge_audio.
    codes_alaw = bytes((i * 17 + 0x55) & 0xFF for i in range(480))
    codes_mulaw = bytes((i * 29 + 0x7F) & 0xFF for i in range(480))
    g711_alaw_48k = make_g711_wave(law="alaw", sample_rate=48_000, channels=1, codes=codes_alaw)
    g711_mulaw_48k = make_g711_wave(law="mulaw", sample_rate=48_000, channels=1, codes=codes_mulaw)
    g711_alaw_empty = make_g711_wave(law="alaw", sample_rate=48_000, channels=1, codes=b"")

    # 3. Ogg Opus, Ogg Vorbis, and Ogg Joint (Vorbis -> Opus) at 48 kHz.
    ogg_opus_stereo = make_opus_link(serial=0x46560208, channels=2)
    ogg_vorbis_stereo = make_vorbis_link(serial=0x46560209, channels=2, sample_rate=48_000)
    ogg_joint_vorbis_opus = (
        make_vorbis_link(serial=0x4656020A, channels=2, sample_rate=48_000)
        + make_opus_link(serial=0x4656020B, channels=2)
    )

    # 4. Chained Vorbis 3ch (0x07) + 4ch (0x33) -> 5ch union mask 0x37;
    #    plus Chained Vorbis 4ch (0x33) + 9ch (discrete mask 0) -> 9ch discrete mask 0 (cannot downmix),
    #    and a Joint chain combining 4ch Vorbis (0x33) with 4ch discrete Opus (mapping family 255, mask 0) -> 4ch mask 0.
    ogg_vorbis_chained_5ch_0x37 = (
        make_vorbis_link(serial=0x4656020C, channels=3, sample_rate=48_000)
        + make_vorbis_link(serial=0x4656020D, channels=4, sample_rate=48_000)
    )
    ogg_joint_chained_5ch_0x37 = (
        ogg_vorbis_chained_5ch_0x37
        + make_opus_link(serial=0x4656020E, channels=2)
    )
    ogg_vorbis_chained_8ch_0x73f = (
        make_vorbis_link(serial=0x4656020F, channels=4, sample_rate=48_000)
        + make_vorbis_link(serial=0x46560210, channels=9, sample_rate=48_000)
    )
    opus_discrete_4ch_head = (
        b"OpusHead"
        + bytes([1, 4])
        + struct.pack("<HIhB", 0, 48_000, 0, 255)
        + bytes([4, 0, 0, 1, 2, 3])
    )
    opus_discrete_4ch_tags = b"OpusTags" + struct.pack("<I", 4) + b"fvid" + struct.pack("<I", 0)
    raw20_4s = (bytes([0xF8, 2, 0xFF, 0xFE]) * 3) + bytes.fromhex("f8fffe")
    opus_discrete_4ch_link = b"".join(
        [
            ogg_page(opus_discrete_4ch_head, 0, 2, 0, 0x46560211),
            ogg_page(opus_discrete_4ch_tags, 0, 0, 1, 0x46560211),
            ogg_page(raw20_4s, 960, 0, 2, 0x46560211),
            ogg_page(raw20_4s, 1920, 4, 3, 0x46560211),
        ]
    )
    ogg_joint_chained_8ch_0x73f = (
        make_vorbis_link(serial=0x4656020F, channels=4, sample_rate=48_000)
        + opus_discrete_4ch_link
    )

    # 5. Bare FLAC (4-channel standard quad 0x33 vs 4-channel unassigned 0x0) and bare MP3 from existing synthetic frames.
    flac_source_4ch = (ROOT / "tests/fixtures/playback-errors/flac-native/extended-lpc-4-1-0-0.flac").read_bytes()
    comment_0x0 = (
        struct.pack("<I", 4)
        + b"fvid"
        + struct.pack("<I", 1)
        + struct.pack("<I", 38)
        + b"waveformatextensible_channel_mask=0X00"
    )
    flac_4ch_unassigned = (
        flac_source_4ch[:4]
        + bytes([0])
        + flac_source_4ch[5:42]
        + bytes([0x84])
        + len(comment_0x0).to_bytes(3, "big")
        + comment_0x0
        + flac_source_4ch[42:]
    )
    mp3_stereo = (ROOT / "tests/fixtures/playback-errors/mpeg-clock-metadata/mp3-48000.mp3").read_bytes()

    files = {
        "video.mkv": owned_eof()["video.mkv"],
        "wave-24in32-valid.wav": wave_24in32_valid,
        "wave-24in32-nonzero-padding.wav": wave_24in32_nonzero_padding,
        "g711-alaw-48k-mono.wav": g711_alaw_48k,
        "g711-mulaw-48k-mono.wav": g711_mulaw_48k,
        "g711-alaw-empty.wav": g711_alaw_empty,
        "ogg-opus-stereo.ogg": ogg_opus_stereo,
        "ogg-vorbis-stereo.ogg": ogg_vorbis_stereo,
        "ogg-joint-vorbis-opus.ogg": ogg_joint_vorbis_opus,
        "ogg-vorbis-chained-5ch-0x37.ogg": ogg_vorbis_chained_5ch_0x37,
        "ogg-joint-chained-5ch-0x37.ogg": ogg_joint_chained_5ch_0x37,
        "ogg-vorbis-chained-8ch-0x73f.ogg": ogg_vorbis_chained_8ch_0x73f,
        "ogg-joint-chained-8ch-0x73f.ogg": ogg_joint_chained_8ch_0x73f,
        "flac-4ch-quad.flac": flac_source_4ch,
        "flac-4ch-unassigned.flac": flac_4ch_unassigned,
        "raw-stereo.mp3": mp3_stereo,
    }
    manifest = {
        "generator": "scripts/generate_wave_ogg_flac_plan_regressions.py",
        "batch_id": "FVID-0208",
        "provenance": "100% synthetic RIFF/WAVE, Ogg Opus/Vorbis/Joint, bare FLAC, and bare MP3 fixtures; no FFmpeg, network, or private media.",
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
