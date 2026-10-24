#!/usr/bin/env python3
"""Generate synthetic fixtures for subtitle conversion & normalize-loudness Matroska metadata regressions."""

import math
import pathlib
import struct


def write_srt(path: pathlib.Path) -> None:
    content = (
        "1\n"
        "00:00:00,100 --> 00:00:01,100\n"
        "<b>First</b> subtitle cue\n"
        "\n"
        "2\n"
        "00:00:01,200 --> 00:00:02,200\n"
        "<i>Second</i> subtitle cue with extra text to exceed small packet byte limits\n"
    )
    path.write_text(content, encoding="utf-8")


def write_ass(path: pathlib.Path) -> None:
    content = (
        "[Script Info]\n"
        "ScriptType: v4.00+\n"
        "PlayResX: 384\n"
        "PlayResY: 288\n"
        "[V4+ Styles]\n"
        "Format: Name, Fontname, Fontsize\n"
        "Style: Default,Arial,18\n"
        "[Events]\n"
        "Format: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\n"
        "Dialogue: 0,0:00:00.10,0:00:01.10,Default,,0,0,0,,{\\b1}ASS first{\\b0}\n"
        "Dialogue: 0,0:00:01.20,0:00:02.20,Default,,0,0,0,,{\\i1}ASS second cue with longer text payload{\\i0}\n"
    )
    path.write_text(content, encoding="utf-8")


def write_pcm_wav_with_info(path: pathlib.Path) -> None:
    sample_rate = 48000
    channels = 1
    frames = 48000  # 1.0 second at 48 kHz (needed for complete 400ms loudness window)
    samples = bytearray()
    for i in range(frames):
        # 1 kHz sine at -12 dBFS (~8230 amplitude in s16)
        val = int(round(8230.0 * math.sin(2.0 * math.pi * 1000.0 * i / sample_rate)))
        samples.extend(struct.pack("<h", val))

    def riff_chunk(fourcc: bytes, payload: bytes) -> bytes:
        pad = b"\x00" if (len(payload) % 2 == 1) else b""
        return fourcc + struct.pack("<I", len(payload)) + payload + pad

    fmt_payload = struct.pack(
        "<HHIIHH",
        1,  # PCM
        channels,
        sample_rate,
        sample_rate * channels * 2,
        channels * 2,
        16,
    )
    info_body = (
        b"INFO"
        + riff_chunk(b"INAM", b"Original Wave Title\x00")
        + riff_chunk(b"IART", b"Original Wave Artist\x00")
    )
    list_chunk = riff_chunk(b"LIST", info_body)
    fmt_chunk = riff_chunk(b"fmt ", fmt_payload)
    data_chunk = riff_chunk(b"data", bytes(samples))
    wave_body = b"WAVE" + fmt_chunk + list_chunk + data_chunk
    riff = b"RIFF" + struct.pack("<I", len(wave_body)) + wave_body
    path.write_bytes(riff)


def main() -> None:
    root = pathlib.Path(__file__).resolve().parents[1]
    out_dir = root / "tests" / "fixtures" / "playback-errors" / "subtitle-normalize-metadata"
    out_dir.mkdir(parents=True, exist_ok=True)
    write_srt(out_dir / "cues-two.srt")
    write_ass(out_dir / "cues-two.ass")
    write_pcm_wav_with_info(out_dir / "tone-1s-48k.wav")


if __name__ == "__main__":
    main()
