#!/usr/bin/env python3
"""Generate synthetic playback controls & runtime thread regression fixtures (FVID-0202)."""

from __future__ import annotations

import json
import struct
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
OUT_DIR = ROOT / "tests" / "fixtures" / "playback-errors" / "play-controls"


def build_two_frame_y4m(width: int = 2, height: int = 2, fps: int = 10) -> bytes:
    """Build a minimal synthetic 2-frame YUV4MPEG2 (Y4M) 4:2:0 stream."""
    header = f"YUV4MPEG2 W{width} H{height} F{fps}:1 Ip C420jpeg\n".encode("ascii")
    frame_a = b"FRAME\n" + bytes([16, 32, 48, 64, 128, 128])
    frame_b = b"FRAME\n" + bytes([80, 96, 112, 128, 128, 128])
    return header + frame_a + frame_b


def build_ass_with_arrow_in_dialogue() -> str:
    return (
        "[Script Info]\n"
        "; Comment with arrow --> in header\n"
        "Title: Synthetic ASS arrow test\n"
        "ScriptType: v4.00+\n"
        "Dialogue: 0,0:00:00.00,0:00:00.50,Default,,0,0,0,,Fake header dialogue\n"
        "\n"
        "[V4+ Styles]\n"
        "Format: Name, Fontname, Fontsize, PrimaryColour\n"
        "Style: Default,Arial,20,&H00FFFFFF\n"
        "\n"
        "[Events]\n"
        "Format: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\n"
        "Dialogue: 0,0:00:01.00,0:00:03.00,Default,,0,0,0,,Step A --> Step B\n"
        "\n"
        "[Aegisub Extradata]\n"
        "Dialogue: 0,0:00:04.00,0:00:05.00,Default,,0,0,0,,Trailing non-event dialogue\n"
    )


def generate(out_dir: Path = OUT_DIR) -> dict:
    out_dir.mkdir(parents=True, exist_ok=True)
    y4m_bytes = build_two_frame_y4m()
    y4m_path = out_dir / "two-frame-420.y4m"
    y4m_path.write_bytes(y4m_bytes)

    ass_text = build_ass_with_arrow_in_dialogue()
    ass_path = out_dir / "arrow-in-dialogue.ass"
    ass_path.write_text(ass_text, encoding="utf-8")

    manifest = {
        "generator": "scripts/generate_play_controls_regressions.py",
        "improvement_id": "FVID-0202",
        "fixtures": [
            {
                "file": "two-frame-420.y4m",
                "size": len(y4m_bytes),
                "description": "Synthetic 2-frame 2x2 Y4M 4:2:0 stream at 10 fps for Playback thread seek-past-EOF single Ended event verification",
            },
            {
                "file": "arrow-in-dialogue.ass",
                "size": len(ass_text.encode("utf-8")),
                "description": "Synthetic ASS subtitle file containing '-->' in header/dialogue and non-Events Dialogue lines",
            },
        ],
    }
    (out_dir / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8")
    return manifest


if __name__ == "__main__":
    generate()
