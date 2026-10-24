#!/usr/bin/env python3
"""Generate synthetic SMF and XM regression fixtures without FFmpeg or network access."""

from __future__ import annotations

import json
import struct
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
OUT_DIR = ROOT / "tests" / "fixtures" / "playback-errors" / "smf-xm"


def vlq(value: int) -> bytes:
    parts = [value & 0x7F]
    value >>= 7
    while value > 0:
        parts.append((value & 0x7F) | 0x80)
        value >>= 7
    return bytes(reversed(parts))


def smf_header(fmt: int, tracks: int, ppq: int) -> bytes:
    return b"MThd" + struct.pack(">IHHH", 6, fmt, tracks, ppq)


def smf_track(events: bytes) -> bytes:
    return b"MTrk" + struct.pack(">I", len(events)) + events


def build_valid_smf() -> bytes:
    events = (
        vlq(0)
        + b"\xff\x51\x03"
        + (500_000).to_bytes(3, "big")
        + vlq(0)
        + b"\x90\x3c\x64"
        + vlq(480)
        + b"\x80\x3c\x00"
        + vlq(0)
        + b"\xff\x2f\x00"
    )
    return smf_header(0, 1, 480) + smf_track(events)


def build_missing_eot_smf() -> bytes:
    events = (
        vlq(0)
        + b"\xff\x51\x03"
        + (500_000).to_bytes(3, "big")
        + vlq(0)
        + b"\x90\x3c\x64"
        + vlq(480)
        + b"\x80\x3c\x00"
    )
    return smf_header(0, 1, 480) + smf_track(events)


def xm_field(text: str, width: int) -> bytes:
    raw = text.encode("ascii")[:width]
    return raw + b"\x00" * (width - len(raw))


def build_xm(
    *,
    packed: bool,
    cell_note: int,
    sixteen_bit: bool = False,
    sample_byte_len: int | None = None,
    declared_packed_size: int | None = None,
    trailing_bytes: bytes = b"",
) -> bytes:
    out = bytearray()
    out += b"Extended Module: "
    out += xm_field("fvid smf xm", 20)
    out.append(0x1A)
    out += xm_field("FastTracker II v2.00", 20)
    out += struct.pack("<H", 0x0104)
    out += struct.pack("<I", 276)  # MIN_HEADER_SIZE
    out += struct.pack("<H", 1)  # song length (orders)
    out += struct.pack("<H", 0)  # restart position
    out += struct.pack("<H", 1)  # channels
    out += struct.pack("<H", 1)  # patterns
    out += struct.pack("<H", 1)  # instruments
    out += struct.pack("<H", 1)  # linear freq table
    out += struct.pack("<H", 6)  # speed
    out += struct.pack("<H", 125)  # bpm
    order_table = bytearray(256)
    order_table[0] = 0
    out += order_table

    # Pattern 0: 1 row, 1 channel
    pattern_data = bytearray()
    if packed:
        pattern_data.append(0x80 | 0x1F)
    pattern_data += bytes([cell_note, 1, 0x40, 0, 0])
    if declared_packed_size is not None:
        pattern_data = pattern_data[:declared_packed_size]
    out += struct.pack("<IBHH", 9, 0, 1, len(pattern_data))
    out += pattern_data

    # Instrument 0: 1 sample
    inst = bytearray()
    inst += struct.pack("<I", 243)
    inst += xm_field("lead", 22)
    inst.append(0)
    inst += struct.pack("<H", 1)  # 1 sample
    inst += struct.pack("<I", 40)  # SAMPLE_RECORD_BYTES
    inst += bytes(96)  # keymap: all 0
    inst += bytes(96)  # envelopes (2 * 48)
    inst += bytes(233 - 225)
    inst += struct.pack("<H", 0)  # envelope flags
    inst += bytes(4)  # auto-vibrato
    inst += struct.pack("<H", 0)  # fadeout
    inst += struct.pack("<H", 0)  # reserved
    assert len(inst) == 243

    if sixteen_bit:
        # 2 frames of 16-bit delta PCM = 4 bytes normally, or overridden
        pcm = struct.pack("<hh", 4096, -8192)
        if sample_byte_len is not None:
            if sample_byte_len > len(pcm):
                pcm = pcm + b"\x00" * (sample_byte_len - len(pcm))
            else:
                pcm = pcm[:sample_byte_len]
        declared_len = len(pcm)
        sample_type = 0x10  # SAMPLE_FLAG_16_BIT
    else:
        pcm = bytes([32, 192, 64, 192])
        declared_len = len(pcm)
        sample_type = 0x00

    sample_rec = bytearray()
    sample_rec += struct.pack("<III", declared_len, 0, 0)
    sample_rec.append(64)  # volume
    sample_rec.append(0)  # finetune
    sample_rec.append(sample_type)
    sample_rec.append(128)  # panning
    sample_rec.append(0)  # relative note
    sample_rec.append(0)  # reserved
    sample_rec += xm_field("s0", 22)
    assert len(sample_rec) == 40

    out += inst
    out += sample_rec
    out += pcm
    out += trailing_bytes
    return bytes(out)


def generate(out_dir: Path = OUT_DIR) -> dict[str, bytes]:
    out_dir.mkdir(parents=True, exist_ok=True)
    files = {
        "valid.mid": build_valid_smf(),
        "missing-eot.mid": build_missing_eot_smf(),
        "packed-note-120.xm": build_xm(packed=True, cell_note=120),
        "unpacked-note-120.xm": build_xm(packed=False, cell_note=120),
        "valid-16bit.xm": build_xm(
            packed=True, cell_note=48, sixteen_bit=True, sample_byte_len=4
        ),
        "odd-16bit-sample.xm": build_xm(
            packed=True, cell_note=48, sixteen_bit=True, sample_byte_len=5
        ),
        "pattern-overrun-into-instrument.xm": build_xm(
            packed=True, cell_note=48, declared_packed_size=2
        ),
    }
    for name, content in files.items():
        (out_dir / name).write_bytes(content)

    manifest = {
        "generator": "scripts/generate_smf_xm_regressions.py",
        "cases": [
            {
                "id": "valid-smf",
                "file": "valid.mid",
                "expect": "accept",
            },
            {
                "id": "missing-eot-smf",
                "file": "missing-eot.mid",
                "expect": "refuse",
                "reason": "SMF track ends without an end-of-track meta event",
            },
            {
                "id": "unpacked-note-120-xm",
                "file": "unpacked-note-120.xm",
                "control": "packed-note-120.xm",
                "expect": "accept-clamped-note-zero",
            },
            {
                "id": "valid-16bit-xm",
                "file": "valid-16bit.xm",
                "expect": "accept",
            },
            {
                "id": "odd-16bit-sample-xm",
                "file": "odd-16bit-sample.xm",
                "expect": "refuse",
                "reason": "16-bit sample states an odd byte length",
            },
            {
                "id": "pattern-overrun-into-instrument-xm",
                "file": "pattern-overrun-into-instrument.xm",
                "expect": "refuse",
                "reason": "pattern rows run past the packed data stated in pattern header",
            },
        ],
    }
    (out_dir / "manifest.json").write_text(
        json.dumps(manifest, indent=2) + "\n", encoding="utf-8"
    )
    return files


if __name__ == "__main__":
    generate()
