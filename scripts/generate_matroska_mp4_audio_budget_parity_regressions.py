#!/usr/bin/env python3
"""Generate deterministic synthetic fixtures for Batch 257 (`FVID-0264`).

Covers:
1. Multitrack Matroska VP9+AAC (`multitrack-vp9-aac.mkv`, track 0 is 16x16 VP9
   with 1024-byte packets; track 1 is 48 kHz mono AAC with ~28-byte packets and
   track title/language metadata) for selected-track Matroska audio export,
   plan_decode_audio, loudnorm, and mix-audio/merge-audio packet-budget checks.
2. Multitrack Matroska VP9+PCM (`multitrack-vp9-pcm.mkv`, track 0 is 16x16 VP9
   with 1024-byte packets; track 1 is 48 kHz mono 16-bit PCM with 16-byte packets)
   for selected-track Matroska PCM audio export and plan_decode_audio packet-budget
   checks.
3. Multitrack MP4 AVC+AAC (`multitrack-avc-aac.mp4`, track 0 is 32x16 AVC I_PCM
   with ~778-byte packets; track 1 is 48 kHz mono AAC with ~28-byte packets) for
   selected-track MP4 audio export, loudnorm, and mix-audio/merge-audio
   packet-budget checks.
"""

from __future__ import annotations

import hashlib
import json
from pathlib import Path
import struct

from generate_ffv1_gray_fixtures import element, uint
from generate_mp4_audio_selection_regressions import (
    build_aac_packet,
    build_esds,
    make_audio_track,
    mp4a_v0_header,
)
from generate_rotated_mp4_decode_export_probe_regressions import (
    build_avc_32x16_ipcm,
    build_rotated_avc_video_track,
)
from generate_vp9_chroma import atom, word


ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / "tests/fixtures/playback-errors/matroska-mp4-audio-budget-parity"


def build_multitrack_matroska_vp9_aac(second_title: str = "Main Audio") -> bytes:
    aac_pkt_0 = build_aac_packet(100)
    aac_pkt_1 = build_aac_packet(115)
    assert len(aac_pkt_0) <= 64 and len(aac_pkt_1) <= 64
    vp9_pkt = b"\x82\x49\x83\x42\x00\x00\xf0\x00\xf6\x00\x38\x24\x1c\x18\x54\x00\x00\x30\x70\x00\x00\x40\x00\xef\x85\xac\x00"
    vp9_pkt = vp9_pkt + bytes(1024 - len(vp9_pkt))
    asc = b"\x11\x88"

    header = element(
        0x1A45DFA3,
        uint(0x4286, 1)
        + uint(0x42F7, 1)
        + uint(0x42F2, 4)
        + uint(0x42F3, 8)
        + element(0x4282, b"matroska")
        + uint(0x4287, 4)
        + uint(0x4285, 2),
    )
    info = element(
        0x1549A966,
        uint(0x2AD7B1, 1_000_000)
        + element(0x4D80, b"fvid-synthetic")
        + element(0x5741, b"fvid-synthetic")
        + element(0x7BA9, b"Synthetic Bundle")
        + element(0x4489, struct.pack(">d", 42.667)),
    )
    v_track = element(
        0xAE,
        uint(0xD7, 1)
        + uint(0x73C5, 1)
        + uint(0x83, 1)
        + element(0x86, b"V_VP9")
        + uint(0x23E383, 40_000_000)
        + element(0xE0, uint(0xB0, 16) + uint(0xBA, 16)),
    )
    a_settings = element(
        0xE1,
        element(0xB5, struct.pack(">d", 48000.0)) + uint(0x9F, 1),
    )
    a_track = element(
        0xAE,
        uint(0xD7, 2)
        + uint(0x73C5, 2)
        + uint(0x83, 2)
        + element(0x536E, second_title.encode("utf-8"))
        + element(0x22B59C, b"eng")
        + element(0x86, b"A_AAC")
        + element(0x63A2, asc)
        + uint(0x23E383, 21_333_334)
        + a_settings,
    )
    tracks = element(0x1654AE6B, v_track + a_track)
    blocks = (
        uint(0xE7, 0)
        + element(0xA3, b"\x81\x00\x00\x80" + vp9_pkt)
        + element(0xA3, b"\x82\x00\x00\x80" + aac_pkt_0)
        + element(0xA3, b"\x81\x00\x15\x80" + vp9_pkt)
        + element(0xA3, b"\x82\x00\x15\x80" + aac_pkt_1)
    )
    cluster = element(0x1F43B675, blocks)
    return header + element(0x18538067, info + tracks + cluster)


def build_multitrack_matroska_vp9_pcm() -> bytes:
    vp9_pkt = b"\x82\x49\x83\x42\x00\x00\xf0\x00\xf6\x00\x38\x24\x1c\x18\x54\x00\x00\x30\x70\x00\x00\x40\x00\xef\x85\xac\x00"
    vp9_pkt = vp9_pkt + bytes(1024 - len(vp9_pkt))
    pcm_pkt_0 = b"".join(
        struct.pack("<h", v)
        for v in (2048, -2048, 4096, -4096, 1024, -1024, 3072, -3072)
    )
    pcm_pkt_1 = b"".join(
        struct.pack("<h", v)
        for v in (1536, -1536, 3584, -3584, 512, -512, 2560, -2560)
    )
    assert len(pcm_pkt_0) == 16 and len(pcm_pkt_1) == 16

    header = element(
        0x1A45DFA3,
        uint(0x4286, 1)
        + uint(0x42F7, 1)
        + uint(0x42F2, 4)
        + uint(0x42F3, 8)
        + element(0x4282, b"matroska")
        + uint(0x4287, 4)
        + uint(0x4285, 2),
    )
    info = element(
        0x1549A966,
        uint(0x2AD7B1, 1_000_000)
        + element(0x4D80, b"fvid-synthetic")
        + element(0x5741, b"fvid-synthetic")
        + element(0x4489, struct.pack(">d", 2.0)),
    )
    v_track = element(
        0xAE,
        uint(0xD7, 1)
        + uint(0x73C5, 1)
        + uint(0x83, 1)
        + element(0x86, b"V_VP9")
        + uint(0x23E383, 40_000_000)
        + element(0xE0, uint(0xB0, 16) + uint(0xBA, 16)),
    )
    a_settings = element(
        0xE1,
        element(0xB5, struct.pack(">d", 8000.0)) + uint(0x9F, 1) + uint(0x6264, 16),
    )
    a_track = element(
        0xAE,
        uint(0xD7, 2)
        + uint(0x73C5, 2)
        + uint(0x83, 2)
        + element(0x86, b"A_PCM/INT/LIT")
        + a_settings,
    )
    tracks = element(0x1654AE6B, v_track + a_track)
    blocks = (
        uint(0xE7, 0)
        + element(0xA3, b"\x81\x00\x00\x80" + vp9_pkt)
        + element(0xA3, b"\x82\x00\x00\x80" + pcm_pkt_0)
        + element(0xA3, b"\x81\x00\x01\x80" + vp9_pkt)
        + element(0xA3, b"\x82\x00\x01\x80" + pcm_pkt_1)
    )
    cluster = element(0x1F43B675, blocks)
    return header + element(0x18538067, info + tracks + cluster)


def build_multitrack_mp4_avc_aac() -> bytes:
    avcc, avc_packets = build_avc_32x16_ipcm()
    aac_pkt_0 = build_aac_packet(100)
    aac_pkt_1 = build_aac_packet(115)
    assert len(aac_pkt_0) <= 64 and len(aac_pkt_1) <= 64
    aac_packets = [aac_pkt_0, aac_pkt_1]

    ftyp = atom(b"ftyp", b"isom\x00\x00\x00\x00isomiso6avc1mp41")
    v_payload = b"".join(avc_packets)
    a_payload = b"".join(aac_packets)
    mdat = atom(b"mdat", v_payload + a_payload)

    v_offset = len(ftyp) + 8
    a_offset = v_offset + len(v_payload)

    asc = b"\x11\x88"
    mp4a_payload = mp4a_v0_header(1, 48000) + atom(b"esds", build_esds(asc))

    v_trak = build_rotated_avc_video_track(
        1, avcc, avc_packets, v_offset, rotation=0, pasp=(1, 1), rate=48000, delta=1024
    )
    a_trak = make_audio_track(2, 3, b"mp4a", mp4a_payload, aac_packets, a_offset, 48000, 1024)

    mvhd = bytearray(100)
    mvhd[12:16] = word(48000)
    mvhd[16:20] = word(2048)
    mvhd[20:24] = word(65536)
    mvhd[24:28] = word(0x01000000)
    for at, val in [(36, 65536), (52, 65536), (68, 1 << 30), (96, 3)]:
        mvhd[at : at + 4] = word(val)
    return ftyp + mdat + atom(b"moov", atom(b"mvhd", bytes(mvhd)) + v_trak + a_trak)


def write_fixture(name: str, payload: bytes, records: list[dict[str, object]]) -> None:
    path = OUT / name
    path.write_bytes(payload)
    records.append(
        {
            "name": name,
            "bytes": len(payload),
            "sha256": hashlib.sha256(payload).hexdigest(),
        }
    )


def main() -> None:
    OUT.mkdir(parents=True, exist_ok=True)
    records: list[dict[str, object]] = []

    write_fixture("multitrack-vp9-aac.mkv", build_multitrack_matroska_vp9_aac("Main Audio"), records)
    write_fixture("multitrack-vp9-aac-alt.mkv", build_multitrack_matroska_vp9_aac("Alt Audio"), records)
    write_fixture("multitrack-vp9-pcm.mkv", build_multitrack_matroska_vp9_pcm(), records)
    write_fixture("multitrack-avc-aac.mp4", build_multitrack_mp4_avc_aac(), records)

    manifest = {
        "generator": "scripts/generate_matroska_mp4_audio_budget_parity_regressions.py",
        "fixtures": records,
    }
    (OUT / "manifest.json").write_text(
        json.dumps(manifest, indent=2) + "\n",
        encoding="utf-8",
    )


if __name__ == "__main__":
    main()
