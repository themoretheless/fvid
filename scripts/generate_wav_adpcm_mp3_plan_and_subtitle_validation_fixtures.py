#!/usr/bin/env python3
"""Generate synthetic fixtures for Batch 581 (FVID-0635..0639):
1. wav-extensible-g722-16bit-storage.wav: WAVEFORMATEXTENSIBLE G.722 ADPCM
   (subformat 0x028F) with invalid container wBitsPerSample=16 and
   wValidBitsPerSample=4.
2. mp3-multi-frame-max-packets.mp3: Bare MPEG-1 Layer III stream with 10 frames
   (11520 samples) used to test plan_mix_audio_with_controls under max_packets=1.
3. mkv-ass-malformed-layer.mkv: Matroska S_TEXT/ASS subtitle stream whose packet
   carries a non-numeric Layer field ("0,Marked=abc,Default,,0,0,0,,Hello").
4. vtt-header-inline-cue.vtt: WebVTT file where the first cue is in the same
   paragraph block as the WEBVTT header ("WEBVTT\\n00:00:00.000 --> 00:00:01.000\\nInline").
5. srt-invalid-clock-fields.srt: SubRip file containing a malformed bare-number
   range ("12,000 --> 15,000") and an out-of-range minute/second clock
   ("00:60:00,000 --> 00:61:00,000") followed by a valid cue.
"""

from pathlib import Path
import struct

OUT_DIR = (
    Path(__file__).resolve().parent.parent
    / "tests"
    / "fixtures"
    / "playback-errors"
    / "wav-adpcm-mp3-plan-and-subtitle-validation"
)


def build_wav_extensible_g722_16bit_storage() -> bytes:
    subformat = struct.pack(
        "<IHH8s",
        0x0000028F,  # G722_ADPCM
        0x0000,
        0x0010,
        b"\x80\x00\x00\xaa\x00\x38\x9b\x71",
    )
    # wFormatTag=0xFFFE, nChannels=1, nSamplesPerSec=16000, nAvgBytesPerSec=8000,
    # nBlockAlign=1, wBitsPerSample=16 (invalid for 4-bit G.722!), cbSize=22,
    # wValidBitsPerSample=4, dwChannelMask=4
    fmt_body = (
        struct.pack(
            "<HHIIHHH",
            0xFFFE,
            1,
            16000,
            8000,
            1,
            16,
            22,
        )
        + struct.pack("<HI", 4, 4)
        + subformat
    )
    assert len(fmt_body) == 40
    fact_body = struct.pack("<I", 16)
    data_body = b"\x00" * 8
    riff_payload = (
        b"WAVE"
        + b"fmt "
        + struct.pack("<I", len(fmt_body))
        + fmt_body
        + b"fact"
        + struct.pack("<I", len(fact_body))
        + fact_body
        + b"data"
        + struct.pack("<I", len(data_body))
        + data_body
    )
    return b"RIFF" + struct.pack("<I", len(riff_payload)) + riff_payload


def build_mp3_multi_frame() -> bytes:
    # MPEG1 Layer III, 32 kbps, 44100 Hz, mono (104 bytes per frame), 50 frames
    # (57600 samples = 230400 bytes of float32 PCM; 1 frame = 1152 samples = 4608 bytes)
    mpeg_frame = bytes([0xFF, 0xFB, 0x10, 0xC4]) + b"\x00" * (104 - 4)
    return mpeg_frame * 50


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


def build_mkv_ass_malformed_layer() -> bytes:
    ebml_head = ebml_el(
        0x1A45DFA3,
        ebml_uint(0x4286, 1)
        + ebml_uint(0x42F7, 1)
        + ebml_uint(0x42F2, 4)
        + ebml_uint(0x42F3, 8)
        + ebml_str(0x4282, "matroska")
        + ebml_uint(0x4287, 2)
        + ebml_uint(0x4285, 2),
    )
    info = ebml_el(
        0x1549A966,
        ebml_uint(0x2AD7B1, 1_000_000) + ebml_str(0x4D80, "test") + ebml_str(0x5741, "test"),
    )
    ass_header = (
        "[Script Info]\nScriptType: v4.00+\n\n"
        "[V4+ Styles]\nFormat: Name, Fontname, Fontsize, PrimaryColour, SecondaryColour, OutlineColour, BackColour, Bold, Italic, Underline, StrikeOut, ScaleX, ScaleY, Spacing, Angle, BorderStyle, Outline, Shadow, Alignment, MarginL, MarginR, MarginV, Encoding\n"
        "Style: Default,Arial,20,&H00FFFFFF,&H000000FF,&H00000000,&H00000000,0,0,0,0,100,100,0,0,1,2,2,2,10,10,10,1\n\n"
        "[Events]\nFormat: ReadOrder, Layer, Style, Name, MarginL, MarginR, MarginV, Effect, Text\n"
    )
    track_entry = ebml_el(
        0xAE,
        ebml_uint(0xD7, 1)
        + ebml_uint(0x73C5, 1)
        + ebml_uint(0x83, 17)
        + ebml_str(0x86, "S_TEXT/ASS")
        + ebml_el(0x63A2, ass_header.encode("utf-8")),
    )
    tracks = ebml_el(0x1654AE6B, track_entry)

    # Malformed ASS block: Layer is "Marked=abc" instead of integer
    bad_event = b"0,Marked=abc,Default,,0,0,0,,Hello"
    block_payload = bytes([0x81, 0x00, 0x00, 0x80]) + bad_event
    block_group = ebml_el(
        0xA0,
        ebml_el(0xA1, block_payload) + ebml_uint(0x9B, 1000),
    )
    cluster = ebml_el(0x1F43B675, ebml_uint(0xE7, 0) + block_group)
    segment = ebml_el(0x18538067, info + tracks + cluster)
    return ebml_head + segment


def build_vtt_header_inline_cue() -> bytes:
    return (
        b"WEBVTT\n"
        b"00:00:00.000 --> 00:00:01.000\n"
        b"Inline header cue\n\n"
        b"00:00:01.000 --> 00:00:02.000\n"
        b"Second cue\n"
    )


def build_srt_invalid_clock_fields() -> bytes:
    return (
        b"1\n"
        b"12,000 --> 15,000\n"
        b"Missing colon\n\n"
        b"2\n"
        b"00:00:60,000 --> 00:01:05,000\n"
        b"Seconds out of range\n\n"
        b"3\n"
        b"00:60:00,000 --> 01:01:00,000\n"
        b"Minutes out of range\n\n"
        b"4\n"
        b"00:00:01,000 --> 00:00:02,000\n"
        b"Valid cue\n"
    )


def main() -> None:
    OUT_DIR.mkdir(parents=True, exist_ok=True)
    (OUT_DIR / "wav-extensible-g722-16bit-storage.wav").write_bytes(
        build_wav_extensible_g722_16bit_storage()
    )
    (OUT_DIR / "mp3-multi-frame-max-packets.mp3").write_bytes(
        build_mp3_multi_frame()
    )
    (OUT_DIR / "mkv-ass-malformed-layer.mkv").write_bytes(
        build_mkv_ass_malformed_layer()
    )
    (OUT_DIR / "vtt-header-inline-cue.vtt").write_bytes(
        build_vtt_header_inline_cue()
    )
    (OUT_DIR / "srt-invalid-clock-fields.srt").write_bytes(
        build_srt_invalid_clock_fields()
    )


if __name__ == "__main__":
    main()
