#!/usr/bin/env python3
"""Generate synthetic fixtures for Batch 584 (FVID-0650..0654):
1. flac-66-tags.flac: Bare FLAC with 64 custom 4-char Vorbis comments (T000..T063)
   followed by TITLE=Canonical FLAC Title and LANGUAGE=deu, testing that canonical
   RIFF INFO tags survive the 64-entry cap.
2. ogg-opus-66-tags.ogg: Ogg Opus with 64 custom 4-char Vorbis comments (T000..T063)
   followed by TITLE=Canonical Opus Title and LANGUAGE=fra, testing that canonical
   RIFF INFO tags survive the 64-entry cap.
3. ogg-opus-long-spool.ogg: Ogg Opus (48 kHz mono) with 12 audio packets (11520
   samples = 240 ms), testing actual prefix_samples in stage-2 WAVE DSP budget
   admission for Ogg exports and plan_decode_audio.
4. flac-long-spool.flac: Bare FLAC (48 kHz mono 16-bit) with 12 frames of 192
   samples (2304 samples), testing actual prefix_frames in stage-2 WAVE DSP
   budget admission for bare FLAC apply.
5. mp3-long-spool.mp3: Bare MPEG-1 Layer III (44100 Hz mono) with 4 audio frames
   (4079 presentation samples), testing actual prefix_samples in stage-2 WAVE DSP
   budget admission for bare MPEG apply.
6. adts-long-spool.aac: ADTS AAC-LC (44100 Hz mono) with 12 silent CCE/SCE frames
   (12288 samples), testing actual prefix_samples in stage-2 WAVE DSP budget
   admission for ADTS plan_decode_audio and apply.
"""

from pathlib import Path
import struct

from generate_audio_plan_mp3_flac_and_matroska_pcm_fixtures import (
    make_flac_constant_frame,
)
from generate_wave_ogg_flac_plan_regressions import ogg_page

OUT_DIR = (
    Path(__file__).resolve().parent.parent
    / "tests"
    / "fixtures"
    / "playback-errors"
    / "flac-ogg-info-cap-and-spool-budget"
)


def build_vorbis_comment_payload(fields: list[str]) -> bytes:
    vendor = b"fvid"
    out = bytearray()
    out += struct.pack("<I", len(vendor)) + vendor
    out += struct.pack("<I", len(fields))
    for field in fields:
        encoded = field.encode("utf-8")
        out += struct.pack("<I", len(encoded)) + encoded
    return bytes(out)


def build_flac_stream(frames: list[bytes], comment_fields: list[str] | None = None) -> bytes:
    sample_rate = 48_000
    channels = 1
    bits_per_sample = 16
    total_samples = 192 * len(frames)
    packed = (
        ((sample_rate & 0xFFFFF) << 44)
        + (((channels - 1) & 0x7) << 41)
        + (((bits_per_sample - 1) & 0x1F) << 36)
        + (total_samples & ((1 << 36) - 1))
    )
    min_frame = min(len(f) for f in frames)
    max_frame = max(len(f) for f in frames)
    si = (
        struct.pack(">HH", 192, 192)
        + min_frame.to_bytes(3, "big")
        + max_frame.to_bytes(3, "big")
        + struct.pack(">Q", packed)
        + b"\x00" * 16
    )
    if comment_fields is None:
        return b"fLaC" + bytes([0x80, 0x00, 0x00, 34]) + si + b"".join(frames)
    vc = build_vorbis_comment_payload(comment_fields)
    return (
        b"fLaC"
        + bytes([0x00, 0x00, 0x00, 34])
        + si
        + bytes([0x84])
        + len(vc).to_bytes(3, "big")
        + vc
        + b"".join(frames)
    )


def build_ogg_opus_stream(
    *,
    serial: int,
    channels: int = 1,
    num_packets: int = 2,
    comment_fields: list[str] | None = None,
) -> bytes:
    head = b"OpusHead" + bytes([1, channels]) + struct.pack("<HIhB", 0, 48_000, 0, 0)
    vc = build_vorbis_comment_payload(comment_fields or [])
    tags = b"OpusTags" + vc
    raw20 = bytes.fromhex("f8fffe")
    pages = [
        ogg_page(head, 0, 2, 0, serial),
        ogg_page(tags, 0, 0, 1, serial),
    ]
    for i in range(num_packets):
        granule = (i + 1) * 960
        flags = 4 if i + 1 == num_packets else 0
        pages.append(ogg_page(raw20, granule, flags, i + 2, serial))
    return b"".join(pages)


def build_mp3_four_frame() -> bytes:
    header = bytes([0xFF, 0xFB, 0x10, 0xC4])
    frame = header + b"\x00" * (104 - 4)
    return frame * 4


def build_adts_frame() -> bytes:
    # AAC-LC (profile=1 in ADTS header -> object_type=2), 44100 Hz (freq_idx=4), mono (chan_cfg=1)
    # Minimal silent SCE payload: ID_SCE (000) + tag (0000) + global_gain (00000000) + ics_reserved(0) + window_sequence(00) + window_shape(0) + max_sfb(000000) + predictor_data_present(0) + pulse_data_present(0) + tns_data_present(0) + gain_control_present(0) + ID_END (111)
    # Or copy the first ADTS frame from tests/fixtures/audio/aac-mono-44k.aac? No, AGENTS.md says generate synthetically!
    # Let's pack the exact bits of a valid silent AAC-LC mono frame:
    # Wait: in AAC-LC:
    # id_syn_ele: 3 bits = 0 (ID_SCE)
    # element_instance_tag: 4 bits = 0
    # global_gain: 8 bits = 0
    # individual_channel_stream:
    #   ics_reserved_bit: 1 bit = 0
    #   window_sequence: 2 bits = 0 (ONLY_LONG_SEQUENCE)
    #   window_shape: 1 bit = 0
    #   max_sfb: 6 bits = 0
    #   predictor_data_present: 1 bit = 0
    #   pulse_data_present: 1 bit = 0
    #   tns_data_present: 1 bit = 0
    #   gain_control_data_present: 1 bit = 0
    #   (section_data: 0 sections because max_sfb == 0)
    #   (scale_factor_data: 0 because max_sfb == 0)
    #   (spectral_data: 0 because max_sfb == 0)
    # id_syn_ele: 3 bits = 7 (ID_END)
    # Total bits = 3 + 4 + 8 + 1 + 2 + 1 + 6 + 1 + 1 + 1 + 1 + 3 = 32 bits = 4 bytes!
    # Bitstream MSB-first:
    # Byte 0: 000 (SCE) 0000 (tag) 0 (gain[7]) -> 0x00
    # Byte 1: 0000000 (gain[6..0]) 0 (ics_res) -> 0x00
    # Byte 2: 00 (win_seq) 0 (win_shape) 00000 (max_sfb[5..1]) -> 0x00
    # Byte 3: 0 (max_sfb[0]) 0 (pred) 0 (pulse) 0 (tns) 0 (gain_ctrl) 111 (ID_END) -> 0x07
    payload = bytes([0x00, 0x00, 0x00, 0x07])
    frame_len = 7 + len(payload)  # 11 bytes
    # ADTS fixed + variable header (56 bits = 7 bytes):
    # syncword: 12 bits = 0xFFF
    # ID: 1 bit = 0 (MPEG-4)
    # layer: 2 bits = 0
    # protection_absent: 1 bit = 1
    # profile: 2 bits = 1 (AAC-LC)
    # sampling_frequency_index: 4 bits = 4 (44100 Hz)
    # private_bit: 1 bit = 0
    # channel_configuration: 3 bits = 1 (mono)
    # original_copy: 1 bit = 0
    # home: 1 bit = 0
    # copyright_id_bit: 1 bit = 0
    # copyright_id_start: 1 bit = 0
    # aac_frame_length: 13 bits = frame_len (11)
    # adts_buffer_fullness: 11 bits = 0x7FF
    # number_of_raw_data_blocks_in_frame: 2 bits = 0
    b0 = 0xFF
    b1 = 0xF1
    b2 = (1 << 6) | (4 << 2) | (0 << 1) | ((1 >> 2) & 1)  # 0x50
    b3 = ((1 & 3) << 6) | ((frame_len >> 11) & 3)  # 0x40
    b4 = (frame_len >> 3) & 0xFF  # 11 >> 3 = 1
    b5 = ((frame_len & 7) << 5) | 0x1F  # (3 << 5) | 0x1F = 0x7F
    b6 = 0xFC
    return bytes([b0, b1, b2, b3, b4, b5, b6]) + payload


def make_flac_constant_frame_utf8(frame_number: int, value: int = 2048) -> bytes:
    if frame_number < 128:
        fn_bytes = bytes([frame_number])
    else:
        fn_bytes = bytes([0xC0 | (frame_number >> 6), 0x80 | (frame_number & 0x3F)])
    header_prefix = bytes([0xFF, 0xF8, 0x1A, 0x08]) + fn_bytes
    crc = 0
    for byte in header_prefix:
        crc ^= byte
        for _ in range(8):
            crc = (((crc << 1) ^ 0x07) if (crc & 0x80) else (crc << 1)) & 0xFF
    header = header_prefix + bytes([crc])
    subframe = b"\x00" + struct.pack(">h", value)
    body = header + subframe
    crc16 = 0
    for byte in body:
        crc16 ^= byte << 8
        for _ in range(8):
            crc16 = (((crc16 << 1) ^ 0x8005) if (crc16 & 0x8000) else (crc16 << 1)) & 0xFFFF
    return body + struct.pack(">H", crc16)


def main() -> None:
    OUT_DIR.mkdir(parents=True, exist_ok=True)
    custom_64 = [f"T{i:03d}=v{i}" for i in range(64)]

    flac_66 = build_flac_stream(
        [make_flac_constant_frame(0, 1024), make_flac_constant_frame(1, 2048)],
        custom_64 + ["TITLE=Canonical FLAC Title", "LANGUAGE=deu"],
    )
    opus_66 = build_ogg_opus_stream(
        serial=0x46560650,
        channels=1,
        num_packets=2,
        comment_fields=custom_64 + ["TITLE=Canonical Opus Title", "LANGUAGE=fra"],
    )
    opus_long = build_ogg_opus_stream(
        serial=0x46560651,
        channels=1,
        num_packets=12,
    )
    flac_long = build_flac_stream(
        [make_flac_constant_frame_utf8(i, 1024) for i in range(240)],
    )
    mp3_long = build_mp3_four_frame()
    adts_long = build_adts_frame() * 30

    (OUT_DIR / "flac-66-tags.flac").write_bytes(flac_66)
    (OUT_DIR / "ogg-opus-66-tags.ogg").write_bytes(opus_66)
    (OUT_DIR / "ogg-opus-long-spool.ogg").write_bytes(opus_long)
    (OUT_DIR / "flac-long-spool.flac").write_bytes(flac_long)
    (OUT_DIR / "mp3-long-spool.mp3").write_bytes(mp3_long)
    (OUT_DIR / "adts-long-spool.aac").write_bytes(adts_long)


if __name__ == "__main__":
    main()
