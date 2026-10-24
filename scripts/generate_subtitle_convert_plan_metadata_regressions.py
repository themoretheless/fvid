#!/usr/bin/env python3
"""Generate synthetic SRT, ASS, and Matroska (S_TEXT/UTF8 + S_TEXT/ASS) fixtures for convert-subtitles planning and WritingApp metadata preservation regressions (FVID-0237)."""

from __future__ import annotations

import hashlib
import json
from pathlib import Path
import struct

from generate_audio_resample_window_fixture import ebml


ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / "tests/fixtures/playback-errors/subtitle-convert-plan-metadata"


def u(tag: str, n: int) -> bytes:
    return ebml(tag, n.to_bytes(max(1, (n.bit_length() + 7) // 8), "big"))


def block_group(track_no: int, rel_ms: int, duration_ms: int, payload: bytes) -> bytes:
    assert 1 <= track_no <= 126
    block = ebml("a1", bytes([0x80 | track_no]) + struct.pack(">hB", rel_ms, 0x80) + payload)
    dur = u("9b", duration_ms)
    return ebml("a0", block + dur)


def simple_tag(name: str, value: str) -> bytes:
    return ebml("67c8", ebml("45a3", name.encode("utf-8")) + ebml("4487", value.encode("utf-8")))


def build_srt() -> bytes:
    return (
        "1\n"
        "00:00:00,100 --> 00:00:01,100\n"
        "<b>First</b> SRT planned cue\n"
        "\n"
        "2\n"
        "00:00:01,250 --> 00:00:02,500\n"
        "<i>Second</i> SRT planned cue with longer text payload for byte budget checks\n"
    ).encode("utf-8")


def build_ass() -> bytes:
    return (
        "[Script Info]\n"
        "ScriptType: v4.00+\n"
        "PlayResX: 384\n"
        "PlayResY: 288\n"
        "[V4+ Styles]\n"
        "Format: Name, Fontname, Fontsize, PrimaryColour, SecondaryColour, OutlineColour, BackColour, Bold, Italic, Underline, StrikeOut, ScaleX, ScaleY, Spacing, Angle, BorderStyle, Outline, Shadow, Alignment, MarginL, MarginR, MarginV, Encoding\n"
        "Style: Default,Arial,16,&H00FFFFFF,&H000000FF,&H00000000,&H00000000,0,0,0,0,100,100,0,0,1,1,0,2,10,10,10,1\n"
        "[Events]\n"
        "Format: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\n"
        "Dialogue: 0,0:00:00.10,0:00:01.10,Default,,0,0,0,,{\\b1}First{\\b0} ASS planned cue\n"
        "Dialogue: 0,0:00:01.25,0:00:02.50,Default,,0,0,0,,{\\i1}Second{\\i0} ASS planned cue with longer text payload for byte budget checks\n"
    ).encode("utf-8")


def build_matroska_subrip() -> bytes:
    header = ebml("1a45dfa3", ebml("4282", b"matroska") + u("4287", 4) + u("4285", 2))
    info = ebml(
        "1549a966",
        u("2ad7b1", 1_000_000) + ebml("5741", b"FVidSubtitleMkvSynth"),
    )
    track = ebml(
        "ae",
        u("d7", 1)
        + u("73c5", 701)
        + u("83", 17)
        + ebml("86", b"S_TEXT/UTF8")
        + ebml("536e", b"MkvSubRipTrack")
        + ebml("22b59c", b"eng"),
    )
    tracks = ebml("1654ae6b", track)
    container_tag = ebml(
        "7373",
        ebml("63c0", b"")
        + simple_tag("TITLE", "SubRip Container Title")
        + simple_tag("ARTIST", "SubRip Container Artist")
        + simple_tag("CUSTOM_SUB_TAG", "SubRipCustomValue"),
    )
    track_tag = ebml(
        "7373",
        ebml("63c0", u("63c5", 701))
        + simple_tag("SUB_TRACK_NOTE", "Track701Note"),
    )
    tags = ebml("1254c367", container_tag + track_tag)
    cluster = ebml(
        "1f43b675",
        u("e7", 0)
        + block_group(1, 100, 1000, b"<b>First</b> Matroska SubRip cue")
        + block_group(
            1,
            1250,
            1250,
            b"<i>Second</i> Matroska SubRip cue with longer text payload for byte budget checks",
        ),
    )
    return header + ebml("18538067", info + tracks + tags + cluster)


def build_matroska_ass() -> bytes:
    ass_header = (
        b"[Script Info]\n"
        b"ScriptType: v4.00+\n"
        b"PlayResX: 384\n"
        b"PlayResY: 288\n"
        b"[V4+ Styles]\n"
        b"Format: Name, Fontname, Fontsize, PrimaryColour, SecondaryColour, OutlineColour, BackColour, Bold, Italic, Underline, StrikeOut, ScaleX, ScaleY, Spacing, Angle, BorderStyle, Outline, Shadow, Alignment, MarginL, MarginR, MarginV, Encoding\n"
        b"Style: Default,Arial,16,&H00FFFFFF,&H000000FF,&H00000000,&H00000000,0,0,0,0,100,100,0,0,1,1,0,2,10,10,10,1\n"
        b"[Events]\n"
        b"Format: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\n"
    )
    header = ebml("1a45dfa3", ebml("4282", b"matroska") + u("4287", 4) + u("4285", 2))
    info = ebml(
        "1549a966",
        u("2ad7b1", 1_000_000) + ebml("5741", b"FVidAssMkvSynth"),
    )
    track = ebml(
        "ae",
        u("d7", 1)
        + u("73c5", 702)
        + u("83", 17)
        + ebml("86", b"S_TEXT/ASS")
        + ebml("536e", b"MkvAssTrack")
        + ebml("22b59c", b"jpn")
        + ebml("63a2", ass_header),
    )
    tracks = ebml("1654ae6b", track)
    container_tag = ebml(
        "7373",
        ebml("63c0", b"")
        + simple_tag("TITLE", "ASS Container Title")
        + simple_tag("CUSTOM_ASS_TAG", "AssCustomValue"),
    )
    tags = ebml("1254c367", container_tag)
    cluster = ebml(
        "1f43b675",
        u("e7", 0)
        + block_group(1, 100, 1000, b"0,0,Default,,0,0,0,,{\\b1}First{\\b0} Matroska ASS cue")
        + block_group(
            1,
            1250,
            1250,
            b"1,0,Default,,0,0,0,,{\\i1}Second{\\i0} Matroska ASS cue with longer text payload",
        ),
    )
    return header + ebml("18538067", info + tracks + tags + cluster)


def generate() -> dict[str, bytes]:
    files = {
        "sample.srt": build_srt(),
        "sample.ass": build_ass(),
        "subrip-source.mkv": build_matroska_subrip(),
        "ass-source.mkv": build_matroska_ass(),
    }
    manifest = {
        "batch_id": "FVID-0237",
        "sha256": {name: hashlib.sha256(data).hexdigest() for name, data in files.items()},
    }
    files["manifest.json"] = (json.dumps(manifest, indent=2, sort_keys=True) + "\n").encode("utf-8")
    return files


def main() -> None:
    OUT.mkdir(parents=True, exist_ok=True)
    for name, data in generate().items():
        (OUT / name).write_bytes(data)


if __name__ == "__main__":
    main()
