#!/usr/bin/env python3
"""Generate synthetic subtitle and filter regression fixtures for Batch 196 (FVID-0203)."""

from __future__ import annotations

import pathlib

ROOT = pathlib.Path(__file__).resolve().parent.parent
OUT_DIR = ROOT / "tests" / "fixtures" / "playback-errors" / "filter-subtitle"


def build_uppercase_entity_and_coords_srt() -> bytes:
    """Synthetic SRT with dot decimal separator, trailing X1/Y1 coordinate tags, and uppercase HTML entities."""
    return (
        "1\n"
        "00:00:01.000 --> 00:00:02.500 X1:100 X2:200 Y1:100 Y2:200\n"
        "R&AMP;D &NBSP; &LT;ok&GT;\n\n"
        "2\n"
        "00:00:03,000 --> 00:00:04,000\n"
        "&QUOT;hello&QUOT; &APOS;world&APOS;\n"
    ).encode("utf-8")


def build_spaced_format_ass() -> bytes:
    """Synthetic ASS script with spaced/case-insensitive Format/Dialogue keys and millisecond precision."""
    return (
        "[Script Info]\n"
        "Title: Synthetic Spaced ASS\n"
        "ScriptType: v4.00+\n"
        "PlayResX: 384\n"
        "PlayResY: 288\n\n"
        "[V4+ Styles]\n"
        "Format: Name, Fontname, Fontsize, PrimaryColour, SecondaryColour, OutlineColour, BackColour, Bold, Italic, Underline, StrikeOut, ScaleX, ScaleY, Spacing, Angle, BorderStyle, Outline, Shadow, Alignment, MarginL, MarginR, MarginV, Encoding\n"
        "Style: Default,Arial,16,&H00FFFFFF,&H000000FF,&H00000000,&H00000000,0,0,0,0,100,100,0,0,1,1,0,2,10,10,10,1\n\n"
        "[Events]\n"
        "format : Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\n"
        "dialogue : 0,0:00:01.250,0:00:02.5,Default,,0,0,0,,Synthetic cue\n"
    ).encode("utf-8")


def generate_all(out_dir: pathlib.Path = OUT_DIR) -> dict[str, pathlib.Path]:
    out_dir.mkdir(parents=True, exist_ok=True)
    files = {
        "uppercase-entity-coords.srt": build_uppercase_entity_and_coords_srt(),
        "spaced-format-millis.ass": build_spaced_format_ass(),
    }
    written: dict[str, pathlib.Path] = {}
    for name, data in files.items():
        path = out_dir / name
        path.write_bytes(data)
        written[name] = path
    return written


if __name__ == "__main__":
    for name, path in generate_all().items():
        print(f"wrote {name} -> {path} ({path.stat().st_size} bytes)")
