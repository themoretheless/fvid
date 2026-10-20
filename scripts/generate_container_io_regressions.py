#!/usr/bin/env python3
"""Authored VP9/EBML and raw Y4M for interrupted container I/O regressions.

No source media, encoder, network or test-time generation is used.
"""
import hashlib
import json
import struct
from pathlib import Path

from generate_audio_resample_window_fixture import ebml
from generate_vp9_chroma import frame

OUT = Path(__file__).resolve().parents[1] / "tests/fixtures/playback-errors"


def main():
    packet = frame(8, "420")
    header = ebml("1a45dfa3", ebml("4282", b"webm"))
    info = ebml("1549a966", ebml("2ad7b1", (1_000_000).to_bytes(3, "big"))
                + ebml("4489", struct.pack(">d", 60.0)))
    video = ebml("e0", ebml("b0", b"\x40") + ebml("ba", b"\x40"))
    track = ebml("ae", ebml("d7", b"\1") + ebml("83", b"\1")
                 + ebml("86", b"V_VP9")
                 + ebml("23e383", (20_000_000).to_bytes(4, "big")) + video)
    tracks = ebml("1654ae6b", track)
    padding = ebml("ec", bytes(128))
    clusters = b"".join(ebml("1f43b675", ebml("e7", bytes([pts]))
                        + ebml("a3", b"\x81\0\0\x80" + packet))
                        for pts in (0, 20, 40))
    data = header + ebml("18538067", info + tracks + padding + clusters)
    # Interrupt seven bytes into Void, while goto is reading skipped payload.
    interrupt_at = len(header) + 8 + len(info) + len(tracks) + 5 + 7
    assert data[interrupt_at:interrupt_at + 16] == bytes(16)
    OUT.mkdir(parents=True, exist_ok=True)
    name = "ebml-interrupted-skip.webm"
    (OUT / name).write_bytes(data)
    (OUT / "ebml-interrupted-skip.json").write_text(json.dumps({
        "file": name, "sha256": hashlib.sha256(data).hexdigest(),
        "interrupt_at": interrupt_at, "width": 64, "height": 64,
        "frames": 3, "pts_ns": [0, 20_000_000, 40_000_000],
        "provenance": "Authored neutral VP9 keyframes and EBML Void. Fault injected by the test reader; no private input or external tools.",
    }, indent=2) + "\n")
    y4m_header = b"YUV4MPEG2 W2 H2 F25:1 Ip C420jpeg\n"
    y4m = y4m_header + b"".join(b"FRAME\n" + bytes([16 + i * 16]) * 4
                                + bytes([128, 128]) for i in range(3))
    y4m_name = "y4m-interrupted-header.y4m"
    (OUT / y4m_name).write_bytes(y4m)
    (OUT / "y4m-interrupted-header.json").write_text(json.dumps({
        "file": y4m_name, "sha256": hashlib.sha256(y4m).hexdigest(),
        "interrupt_at": [0, 8, len(y4m_header), len(y4m_header) + 3,
                         len(y4m_header) + 12, len(y4m)],
        "frames": 3, "width": 2, "height": 2,
        "provenance": "Three authored raw progressive YUV420 pictures; no source media or external tools.",
    }, indent=2) + "\n")
    copy_data = header + ebml("18538067", info + tracks + padding + clusters
                              + ebml("ec", b"authored copy EOF regression"))
    copy_name = "matroska-interrupted-copy.webm"
    (OUT / copy_name).write_bytes(copy_data)
    (OUT / "matroska-interrupted-copy.json").write_text(json.dumps({
        "file": copy_name, "sha256": hashlib.sha256(copy_data).hexdigest(),
        "interrupt_at": len(copy_data), "frames": 3,
        "provenance": "Authored VP9 frames and trailing in-segment Void; interruption injected at EOF verification after identity copy.",
    }, indent=2) + "\n")


if __name__ == "__main__":
    main()
