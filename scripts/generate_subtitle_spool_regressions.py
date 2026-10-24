#!/usr/bin/env python3
"""Generate synthetic subtitle and spool regression fixtures without external tools."""

from __future__ import annotations

import pathlib


ROOT = pathlib.Path(__file__).resolve().parents[1]
OUT_DIR = ROOT / "tests" / "fixtures" / "playback-errors" / "subtitles"


def build_webvtt_comments_and_styles() -> bytes:
    return (
        b"WEBVTT\n\n"
        b"STYLE\n"
        b"::cue {\n"
        b"  /* comment: 00:00:00.100 --> 00:00:00.900 */\n"
        b"  color: yellow;\n"
        b"}\n\n"
        b"NOTE multi-line comment\n"
        b"00:00:00.200 --> 00:00:00.800\n"
        b"hidden note line\n\n"
        b"1\n"
        b"00:00:01.000 --> 00:00:02.500\n"
        b"Visible WebVTT line\n"
    )


def build_sami_uppercase_nbsp_clear() -> bytes:
    return (
        b"<SAMI>\n"
        b"<BODY>\n"
        b"<SYNC Start=1000><P Class=ENCC>First SAMI line</P>\n"
        b"<SYNC Start=2200><P Class=ENCC>&NBSP;</P>\n"
        b"<SYNC Start=3500><P Class=ENCC>Second SAMI line</P>\n"
        b"<SYNC Start=4800><P Class=ENCC>&Nbsp;</P>\n"
        b"</BODY>\n"
        b"</SAMI>\n"
    )


def build_ass_script_info_and_custom_format() -> bytes:
    return (
        b"[Script Info]\n"
        b"Dialogue: 0,0:00:00.10,0:00:00.90,Default,,0,0,0,,Fake header cue\n\n"
        b"[Events]\n"
        b"Format: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\n"
        b"Dialogue: 0,0:00:01.50,0:00:03.00,Default,,0,0,0,,Real ASS cue\n"
        b"\n[Aegisub Project Garbage]\n"
        b"Dialogue: 0,0:00:04.00,0:00:05.00,Default,,0,0,0,,Fake trailing cue\n"
    )


def main() -> None:
    OUT_DIR.mkdir(parents=True, exist_ok=True)
    (OUT_DIR / "webvtt_note_and_style.vtt").write_bytes(build_webvtt_comments_and_styles())
    (OUT_DIR / "sami_uppercase_nbsp.smi").write_bytes(build_sami_uppercase_nbsp_clear())
    (OUT_DIR / "ass_script_info_dialogue.ass").write_bytes(build_ass_script_info_and_custom_format())


if __name__ == "__main__":
    main()
