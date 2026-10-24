#!/usr/bin/env python3
"""Generate deterministic synthetic fixtures for Batch 240 (`FVID-0247`).

Covers:
1. Single-track Matroska audio (`matroska-aac-48k.mka`, 256-byte packets) for
   `fvid::media::plan_remux`, `fvid_media::plan_remux`, `fvid::media::remux`,
   and `fvid media plan remux` / `fvid media remux` with custom `--max-packet-bytes`
   and `--max-rss-mib`.
2. Single-track AAC MP4 (`mp4-aac-48k.m4a`, `mdat` before `moov`) for MP4 -> Matroska
   (`.mka`/`.mkv`) remux and MP4 fast-start relocation (`.mp4`/`.m4a`) with
   `--streams 0`, `--max-packets`, `--max-packet-bytes`, `--max-rss-mib`, and
   Matroska metadata mutations.
3. Single-track PCM MP4 (`mp4-pcm-48k.m4a`, `mdat` before `moov`) for MP4 fast-start
   relocation (`remux` and `plan_remux`) with `--max-rss-mib`.
"""

from __future__ import annotations

import hashlib
import json
from pathlib import Path
import struct

from generate_audio_resample_window_fixture import ebml
from generate_loudnorm_budget_mix_concat_regressions import build_mp4_pcm_m4a


ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / "tests/fixtures/playback-errors/matroska-mp4-remux-budget-parity"


def u(tag: str, n: int) -> bytes:
    return ebml(tag, n.to_bytes(max(1, (n.bit_length() + 7) // 8), "big"))


def simple_block(track_no: int, rel_ms: int, flags: int, payload: bytes) -> bytes:
    assert 1 <= track_no <= 126
    return ebml("a3", bytes([0x80 | track_no]) + struct.pack(">hB", rel_ms, flags) + payload)


def box(kind: bytes, data: bytes) -> bytes:
    return struct.pack(">I4s", len(data) + 8, kind) + data


def build_aac_raw_frame() -> bytes:
    """Raw AAC-LC mono 48 kHz silent frame without ADTS header."""
    fields = [
        (0, 3),
        (0, 4),
        (100, 8),
        (0, 1),
        (0, 2),
        (0, 1),
        (0, 6),
        (0, 1),
        (0, 1),
        (0, 1),
        (0, 1),
        (7, 3),
    ]
    bits = "".join(format(value, f"0{width}b") for value, width in fields)
    bits += "0" * (-len(bits) % 8)
    return int(bits, 2).to_bytes(len(bits) // 8, "big")


def build_matroska_aac_mka() -> bytes:
    """Single-track Matroska A_AAC with 3 packets of 64 bytes each."""
    raw_aac = build_aac_raw_frame()
    padded_pkt = raw_aac + bytes(64 - len(raw_aac))
    asc = bytes([0x11, 0x88])  # AAC-LC, 48 kHz, mono

    header = ebml("1a45dfa3", ebml("4282", b"matroska") + u("4287", 4) + u("4285", 2))
    info = ebml("1549a966", u("2ad7b1", 1_000_000) + ebml("4489", struct.pack(">d", 64.0)))
    audio_settings = ebml("e1", ebml("b5", struct.pack(">d", 48000.0)) + u("9f", 1))
    track = ebml(
        "ae",
        u("d7", 1)
        + u("73c5", 301)
        + u("83", 2)
        + ebml("86", b"A_AAC")
        + ebml("63a2", asc)
        + audio_settings,
    )
    tracks = ebml("1654ae6b", track)
    blocks = u("e7", 0)
    for i in range(3):
        blocks += simple_block(1, i * 21, 0x80, padded_pkt)
    return header + ebml("18538067", info + tracks + ebml("1f43b675", blocks))


def build_mp4_aac_m4a() -> bytes:
    """Single-track MP4 AAC-LC (48 kHz mono, 3 packets of 1024 samples) with mdat before moov."""
    pkt = build_aac_raw_frame()
    packets = [pkt, pkt, pkt]
    payload = b"".join(packets)
    mdat = box(b"mdat", payload)

    asc = bytes([0x11, 0x88])
    dec_specific = bytes([0x05, len(asc)]) + asc
    dec_config_body = (
        bytes([0x40, 0x15, 0x00, 0x00, 0x00])
        + struct.pack(">II", 64000, 64000)
        + dec_specific
    )
    dec_config = bytes([0x04, len(dec_config_body)]) + dec_config_body
    sl_config = bytes([0x06, 0x01, 0x02])
    es_body = struct.pack(">HB", 1, 0) + dec_config + sl_config
    esds_payload = struct.pack(">I", 0) + bytes([0x03, len(es_body)]) + es_body
    esds = box(b"esds", esds_payload)

    mp4a_entry = bytearray(28)
    struct.pack_into(">H", mp4a_entry, 6, 1)
    struct.pack_into(">HH", mp4a_entry, 16, 1, 16)
    struct.pack_into(">I", mp4a_entry, 24, 48_000 << 16)
    mp4a = box(b"mp4a", bytes(mp4a_entry) + esds)

    stsd = box(b"stsd", struct.pack(">II", 0, 1) + mp4a)
    stts = box(b"stts", struct.pack(">IIII", 0, 1, 3, 1024))
    stsc = box(b"stsc", struct.pack(">IIIII", 0, 1, 1, 3, 1))
    stsz = box(b"stsz", struct.pack(">III", 0, len(pkt), 3))
    stco = box(b"stco", struct.pack(">III", 0, 1, 8))
    stbl = box(b"stbl", stsd + stts + stsc + stsz + stco)

    clock = bytearray(24)
    struct.pack_into(">II", clock, 12, 48_000, 3 * 1024)
    hdlr = box(b"hdlr", bytes(8) + b"soun" + bytes(12))
    dinf = box(b"dinf", box(b"dref", struct.pack(">II", 0, 1) + box(b"url ", struct.pack(">I", 1))))
    mdia = box(
        b"mdia",
        box(b"mdhd", bytes(clock)) + hdlr + box(b"minf", box(b"smhd", bytes(8)) + dinf + stbl),
    )

    tkhd = bytearray(84)
    tkhd[3] = 3
    struct.pack_into(">I", tkhd, 12, 1)
    struct.pack_into(">I", tkhd, 20, 3 * 1024)
    struct.pack_into(">H", tkhd, 36, 256)
    for off, val in [(40, 1 << 16), (56, 1 << 16), (72, 1 << 30)]:
        struct.pack_into(">I", tkhd, off, val)
    trak = box(b"trak", box(b"tkhd", bytes(tkhd)) + mdia)

    mvhd = bytearray(100)
    struct.pack_into(">II", mvhd, 12, 48_000, 3 * 1024)
    struct.pack_into(">IH", mvhd, 20, 1 << 16, 256)
    for off, val in [(36, 1 << 16), (52, 1 << 16), (68, 1 << 30), (96, 2)]:
        struct.pack_into(">I", mvhd, off, val)

    return mdat + box(b"moov", box(b"mvhd", bytes(mvhd)) + trak)


def generate_all(out_dir: Path = OUT) -> dict[str, bytes]:
    out_dir.mkdir(parents=True, exist_ok=True)
    fixtures = {
        "matroska-aac-48k.mka": build_matroska_aac_mka(),
        "mp4-aac-48k.m4a": build_mp4_aac_m4a(),
        "mp4-pcm-48k.m4a": build_mp4_pcm_m4a(),
    }
    manifest_entries = []
    for name, data in fixtures.items():
        (out_dir / name).write_bytes(data)
        manifest_entries.append(
            {
                "name": name,
                "bytes": len(data),
                "sha256": hashlib.sha256(data).hexdigest(),
            }
        )
    manifest = {
        "batch": 240,
        "id": "FVID-0247",
        "fixtures": manifest_entries,
    }
    (out_dir / "manifest.json").write_text(
        json.dumps(manifest, indent=2) + "\n",
        encoding="utf-8",
    )
    return fixtures


if __name__ == "__main__":
    generate_all()
