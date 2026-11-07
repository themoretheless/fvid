#!/usr/bin/env python3
"""Generate synthetic fixtures for Batch 577 (FVID-0615..0619)."""

import os
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
OUT = ROOT / "tests" / "fixtures" / "playback-errors" / "subtitles-case-entities-dvbsub-and-y4m-probe"


def ebml_vint(val: int, width: int = 0) -> bytes:
    if width == 0:
        for w in range(1, 9):
            if val < (1 << (7 * w)) - 1:
                width = w
                break
    out = bytearray(width)
    for i in range(width - 1, -1, -1):
        out[i] = val & 0xFF
        val >>= 8
    out[0] |= 1 << (8 - width)
    return bytes(out)


def ebml_elem(elem_id: bytes, payload: bytes) -> bytes:
    return elem_id + ebml_vint(len(payload)) + payload


def ebml_uint(elem_id: bytes, val: int) -> bytes:
    if val == 0:
        return ebml_elem(elem_id, b"\x00")
    raw = bytearray()
    while val > 0:
        raw.append(val & 0xFF)
        val >>= 8
    raw.reverse()
    return ebml_elem(elem_id, bytes(raw))


def make_dvbsub_mkv() -> bytes:
    header = ebml_elem(
        b"\x1a\x45\xdf\xa3",
        ebml_uint(b"\x42\x86", 1)
        + ebml_uint(b"\x42\xf7", 1)
        + ebml_uint(b"\x42\xf2", 4)
        + ebml_uint(b"\x42\xf3", 8)
        + ebml_elem(b"\x42\x82", b"matroska")
        + ebml_uint(b"\x42\x87", 4)
        + ebml_uint(b"\x42\x85", 2),
    )
    info = ebml_elem(
        b"\x15\x49\xa9\x66",
        ebml_uint(b"\x2a\xd7\xb1", 1_000_000)
        + ebml_elem(b"\x4d\x80", b"fvid-test")
        + ebml_elem(b"\x57\x41", b"fvid-test"),
    )
    # Track 1: S_DVBSUB (TrackType 17 = subtitle), 5-byte CodecPrivate (page_id=1, ancillary_page_id=1)
    track_entry = ebml_elem(
        b"\xae",
        ebml_uint(b"\xd7", 1)
        + ebml_uint(b"\x73\xc5", 1)
        + ebml_uint(b"\x83", 17)
        + ebml_elem(b"\x86", b"S_DVBSUB")
        + ebml_elem(b"\x63\xa2", b"\x00\x01\x00\x01"),
    )
    tracks = ebml_elem(b"\x16\x54\xae\x6b", track_entry)
    # Minimal SimpleBlock on track 1 at 0 ms
    simple_block = ebml_elem(b"\xa3", b"\x81\x00\x00\x80" + b"\x20\x00\x0f\x10\x00\x01\x00\x02\x00\x00\xff")
    cluster = ebml_elem(
        b"\x1f\x43\xb6\x75",
        ebml_uint(b"\xe7", 0) + simple_block,
    )
    segment = ebml_elem(b"\x18\x53\x80\x67", info + tracks + cluster)
    return header + segment


def main() -> None:
    OUT.mkdir(parents=True, exist_ok=True)

    # 1. Lowercase [events] ASS sidecar
    ass_lower = (
        "[Script Info]\n"
        "ScriptType: v4.00+\n"
        "\n"
        "[events]\n"
        "Format: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\n"
        "Dialogue: 0,0:00:01.00,0:00:03.00,Default,,0,0,0,,Rock &amp; Roll&#39;s &lt;best&gt;\n"
    )
    (OUT / "lowercase_events.ass").write_text(ass_lower, encoding="utf-8")

    # 2. Mixed-case WebVTT header and NOTE/STYLE/REGION blocks + HTML entities
    vtt_mixed = (
        "webvtt\n\n"
        "note ignored comment block\n"
        "00:00:00.100 --> 00:00:00.500\n"
        "should not appear\n\n"
        "style\n"
        "::cue { color: yellow; }\n\n"
        "region\n"
        "id:r1\n\n"
        "00:00:01.000 --> 00:00:02.500\n"
        "Tom &amp; Jerry: &quot;Hi&#x21;&quot;&nbsp;forever\n"
    )
    (OUT / "mixed_case_blocks.vtt").write_text(vtt_mixed, encoding="utf-8")

    # 3. SRT with HTML entities
    srt_entities = (
        "1\n"
        "00:00:00,500 --> 00:00:02,000\n"
        "<i>AT&amp;T &lt;5G&gt; &quot;Pro&quot; &#39;26&#x27;</i>\n"
    )
    (OUT / "entities.srt").write_text(srt_entities, encoding="utf-8")

    # 4. Matroska S_DVBSUB fixture
    (OUT / "dvbsub_track.mkv").write_bytes(make_dvbsub_mkv())

    # 5. Y4M with invalid pixel aspect A1:0 and invalid XCOLORRANGE=BOGUS
    frame_420_2x2 = b"FRAME\n" + bytes([16, 16, 16, 16, 128, 128])
    (OUT / "invalid_aspect.y4m").write_bytes(
        b"YUV4MPEG2 W2 H2 F25:1 Ip A1:0 C420\n" + frame_420_2x2
    )
    (OUT / "invalid_colorrange.y4m").write_bytes(
        b"YUV4MPEG2 W2 H2 F25:1 Ip A1:1 C420 XCOLORRANGE=BOGUS\n" + frame_420_2x2
    )


if __name__ == "__main__":
    main()
