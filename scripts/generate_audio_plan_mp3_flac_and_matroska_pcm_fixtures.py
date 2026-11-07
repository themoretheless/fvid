#!/usr/bin/env python3
"""Generate synthetic fixtures for Batch 582 (FVID-0640..0644):
1. mp3-info-prefix-interval.mp3: Bare MPEG-1 Layer III stream at 44100 Hz mono
   with a leading LAME Info frame and 4 audio frames (4608 coded samples), used
   to test standalone_packet_work (counting the Info packet and active presentation
   window) and open_with_interval under max_packets.
2. flac-two-frame-prefix-interval.flac: Bare FLAC stream at 48000 Hz mono 16-bit
   with 2 frames of 192 samples each (384 total samples = 8 ms), used to test
   open_with_interval rejecting transform.interval beyond the max_packets prefix.
3. mka-pcm-language-ietf.mka: Matroska A_PCM/INT/LIT audio stream at 48000 Hz
   mono 16-bit (1920 samples = 40 ms) with Language="und" and LanguageIETF="pt-BR",
   used to test Matroska PCM geometry_for_decode non-empty interval validation
   and WAVE export ILNG metadata preservation from LanguageIETF.
"""

from pathlib import Path
import struct

OUT_DIR = (
    Path(__file__).resolve().parent.parent
    / "tests"
    / "fixtures"
    / "playback-errors"
    / "audio-plan-mp3-flac-and-matroska-pcm"
)


def mp3_crc16(data: bytes) -> int:
    crc = 0
    for byte in data:
        crc ^= byte
        for _ in range(8):
            crc = (crc >> 1) ^ (0xA001 if (crc & 1) else 0)
    return crc


def build_mp3_info_prefix_interval() -> bytes:
    # MPEG-1 Layer III, 32 kbps, 44100 Hz, mono -> 104 bytes per frame, 17-byte side info
    header = bytes([0xFF, 0xFB, 0x10, 0xC4])
    frame_size = 104
    side_bytes = 17
    coded_frames = 4
    encoder_delay = 0
    # Trim the final frame via end_padding = 1800 (with decoder delay 529, tail = 1800 - 529 = 1271
    # samples trimmed from 4608 -> presentation_window = 529..3337, so frames 0, 1, 2 are active
    # and frame 3 at pts 3456 >= 3337 is past presentation_window.end!)
    end_padding = 1800

    info_buf = bytearray(frame_size)
    info_buf[:4] = header
    offset = 4 + side_bytes
    info_buf[offset : offset + 4] = b"Info"
    struct.pack_into(">I", info_buf, offset + 4, 1)  # flags = 1 (frames field present)
    struct.pack_into(">I", info_buf, offset + 8, coded_frames)
    at = offset + 12
    tag = bytearray(36)
    tag[:9] = b"LAME3.100"
    tag[9] = 0x00
    tag[21] = (encoder_delay >> 4) & 0xFF
    tag[22] = ((encoder_delay & 0x0F) << 4) | ((end_padding >> 8) & 0x0F)
    tag[23] = end_padding & 0xFF
    info_buf[at : at + 34] = tag[:34]
    crc = mp3_crc16(bytes(info_buf[: at + 34]))
    struct.pack_into(">H", info_buf, at + 34, crc)

    audio_frame = header + b"\x00" * (frame_size - 4)
    return bytes(info_buf) + audio_frame * coded_frames


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


def make_flac_constant_frame(frame_number: int, value: int = 2048) -> bytes:
    header_prefix = bytes([0xFF, 0xF8, 0x1A, 0x08, frame_number])
    header = header_prefix + bytes([crc8(header_prefix)])
    subframe = b"\x00" + struct.pack(">h", value)
    body = header + subframe
    return body + struct.pack(">H", crc16_flac(body))


def build_flac_two_frame() -> bytes:
    f0 = make_flac_constant_frame(0, value=1024)
    f1 = make_flac_constant_frame(1, value=2048)
    sample_rate = 48_000
    channels = 1
    bits_per_sample = 16
    total_samples = 384
    packed = (
        ((sample_rate & 0xFFFFF) << 44)
        + (((channels - 1) & 0x7) << 41)
        + (((bits_per_sample - 1) & 0x1F) << 36)
        + (total_samples & ((1 << 36) - 1))
    )
    si = (
        struct.pack(">HH", 192, 192)
        + len(f0).to_bytes(3, "big")
        + len(f1).to_bytes(3, "big")
        + struct.pack(">Q", packed)
        + b"\x00" * 16
    )
    return b"fLaC" + bytes([0x80, 0x00, 0x00, 34]) + si + f0 + f1


def ebml_id(raw: int) -> bytes:
    length = (raw.bit_length() + 7) // 8
    return raw.to_bytes(length, "big")


def ebml_vint(value: int) -> bytes:
    for width in range(1, 9):
        limit = (1 << (7 * width)) - 1
        if value < limit:
            marker = 1 << (7 * width)
            return (marker | value).to_bytes(width, "big")
    raise ValueError("vint overflow")


def ebml_el(tag: int, payload: bytes) -> bytes:
    return ebml_id(tag) + ebml_vint(len(payload)) + payload


def ebml_uint(tag: int, value: int) -> bytes:
    if value == 0:
        return ebml_el(tag, b"\x00")
    length = (value.bit_length() + 7) // 8
    return ebml_el(tag, value.to_bytes(length, "big"))


def ebml_str(tag: int, text: str) -> bytes:
    return ebml_el(tag, text.encode("utf-8"))


def build_mka_pcm_language_ietf() -> bytes:
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
    # Track 1: A_PCM/INT/LIT, Language (0x22B59C) = "und", LanguageIETF (0x22B59D) = "pt-BR"
    track_entry = ebml_el(
        0xAE,
        ebml_uint(0xD7, 1)
        + ebml_uint(0x73C5, 1)
        + ebml_uint(0x83, 2)
        + ebml_str(0x86, "A_PCM/INT/LIT")
        + ebml_str(0x22B59C, "und")
        + ebml_str(0x22B59D, "pt-BR")
        + audio_settings,
    )
    tracks = ebml_el(0x1654AE6B, track_entry)

    # 1920 samples (40 ms at 48 kHz) of 16-bit little-endian PCM
    pcm_payload = b"".join(
        struct.pack("<h", 4096 if (i % 32) < 16 else -4096) for i in range(1920)
    )
    simple_block = ebml_el(
        0xA3,
        b"\x81\x00\x00\x80" + pcm_payload,
    )
    cluster = ebml_el(0x1F43B675, ebml_uint(0xE7, 0) + simple_block)
    segment = ebml_el(0x18538067, info + tracks + cluster)
    return ebml_head + segment


def main() -> None:
    OUT_DIR.mkdir(parents=True, exist_ok=True)
    (OUT_DIR / "mp3-info-prefix-interval.mp3").write_bytes(
        build_mp3_info_prefix_interval()
    )
    (OUT_DIR / "flac-two-frame-prefix-interval.flac").write_bytes(
        build_flac_two_frame()
    )
    (OUT_DIR / "mka-pcm-language-ietf.mka").write_bytes(
        build_mka_pcm_language_ietf()
    )


if __name__ == "__main__":
    main()
