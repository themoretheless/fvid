#!/usr/bin/env python3
"""Generate synthetic WavPack, MP3, and AC-3 regression fixtures without FFmpeg or network access."""

from __future__ import annotations

import json
import struct
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "scripts"))

from generate_wavpack_false_stereo import block as wavpack_block
from generate_wavpack_float import metadata as wavpack_metadata, parts as wavpack_parts, rewrite as wavpack_rewrite

OUT_DIR = ROOT / "tests" / "fixtures" / "playback-errors" / "wavpack-mp3-ac3"


def build_valid_wavpack() -> bytes:
    raw, _pcm = wavpack_block(16, 8, False, 160 * 50, mode="mono")
    flags = (int.from_bytes(raw[24:28], "little") & ~(15 << 23)) | (1 << 23)
    raw = wavpack_rewrite(raw, [(t, v) for t, v in wavpack_parts(raw) if t != 0x27], flags)
    b = bytearray(raw)
    b[12:16] = (160).to_bytes(4, "little")
    b[16:20] = (0).to_bytes(4, "little")
    return bytes(b)


def build_wavpack_ape_valid() -> bytes:
    raw = build_valid_wavpack()
    item = struct.pack("<II", 5, 0) + b"Title\x00owned"
    flags = 0x80000000
    header = b"APETAGEX" + struct.pack("<IIII", 2000, 32 + len(item), 1, flags | 0x20000000) + bytes(8)
    footer = b"APETAGEX" + struct.pack("<IIII", 2000, 32 + len(item), 1, flags) + bytes(8)
    return raw + header + item + footer


def build_wavpack_ape_header_no_flag() -> bytes:
    raw = build_valid_wavpack()
    item = struct.pack("<II", 5, 0) + b"Title\x00owned"
    # Header has bit 29 (is_header) set, but bit 31 (has_header) cleared!
    header = b"APETAGEX" + struct.pack("<IIII", 2000, 32 + len(item), 1, 0x20000000) + bytes(8)
    footer = b"APETAGEX" + struct.pack("<IIII", 2000, 32 + len(item), 1, 0x80000000) + bytes(8)
    return raw + header + item + footer


def build_wavpack_ape_footer_no_footer_flag() -> bytes:
    raw = build_valid_wavpack()
    item = struct.pack("<II", 5, 0) + b"Title\x00owned"
    # Footer has bit 30 (no_footer = 0x40000000) set!
    footer = b"APETAGEX" + struct.pack("<IIII", 2000, 32 + len(item), 1, 0x40000000) + bytes(8)
    return raw + item + footer


def build_wavpack_trailing_subblock_byte() -> bytes:
    raw = bytearray(build_valid_wavpack())
    # Append 1 trailing byte inside the block so Block::parse leaves 1 unread byte at the end
    raw.append(0x00)
    size = len(raw) - 8
    raw[4:8] = size.to_bytes(4, "little")
    return bytes(raw)


def generate(out_dir: Path = OUT_DIR) -> dict[str, bytes]:
    out_dir.mkdir(parents=True, exist_ok=True)
    files = {
        "valid.wv": build_wavpack_ape_valid(),
        "ape-header-no-flag.wv": build_wavpack_ape_header_no_flag(),
        "ape-footer-no-footer-flag.wv": build_wavpack_ape_footer_no_footer_flag(),
        "trailing-subblock-byte.wv": build_wavpack_trailing_subblock_byte(),
    }
    for name, content in files.items():
        (out_dir / name).write_bytes(content)

    manifest = {
        "generator": "scripts/generate_wavpack_mp3_ac3_regressions.py",
        "cases": [
            {
                "id": "valid-wavpack-ape",
                "file": "valid.wv",
                "expect": "accept",
            },
            {
                "id": "ape-header-no-flag",
                "file": "ape-header-no-flag.wv",
                "expect": "refuse",
                "reason": "APEv2 header disagrees with footer flags",
            },
            {
                "id": "ape-footer-no-footer-flag",
                "file": "ape-footer-no-footer-flag.wv",
                "expect": "refuse",
                "reason": "APEv2 footer sets no-footer bit (bit 30)",
            },
            {
                "id": "trailing-subblock-byte",
                "file": "trailing-subblock-byte.wv",
                "expect": "refuse",
                "reason": "WavPack block has trailing byte after metadata items",
            },
        ],
    }
    (out_dir / "manifest.json").write_text(
        json.dumps(manifest, indent=2) + "\n", encoding="utf-8"
    )
    return files


if __name__ == "__main__":
    generate()
