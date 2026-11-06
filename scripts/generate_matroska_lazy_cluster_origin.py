#!/usr/bin/env python3
"""Authored multi-cluster VP9 and AV1 Matroska/WebM clips with declared Segment.Info.Duration
where the opening cluster holds only companion subtitle packets or invisible video pre-roll.
"""
import hashlib
import json
import struct
from pathlib import Path
from generate_audio_resample_window_fixture import ebml
from generate_vp9_chroma import frame as vp9_frame
from generate_av1_hdr_sequence import generate as generate_av1_hdr, record_from_seed, SEED

ROOT = Path(__file__).resolve().parents[1] / "tests/fixtures/playback-errors"
OUT = ROOT / "matroska-lazy-cluster-origin"


def vp9_clip(preamble_mode, origin_ms, frame_count, step_ms):
    header = ebml("1a45dfa3", ebml("4282", b"matroska") + ebml("4287", b"\4") + ebml("4285", b"\4"))
    declared_ms = origin_ms + frame_count * step_ms
    info = (
        ebml("2ad7b1", (1_000_000).to_bytes(3, "big"))
        + ebml("4489", struct.pack(">d", float(declared_ms)))
    )
    video_track = (
        ebml("d7", b"\1")
        + ebml("73c5", b"\1")
        + ebml("83", b"\1")
        + ebml("86", b"V_VP9")
        + ebml("23e383", (step_ms * 1_000_000).to_bytes(4, "big"))
        + ebml("e0", ebml("b0", b"\x10") + ebml("ba", b"\x10"))
    )
    tracks = ebml("ae", video_track)
    if preamble_mode == "subtitle":
        sub_track = ebml("d7", b"\2") + ebml("73c5", b"\2") + ebml("83", b"\x11") + ebml("86", b"S_TEXT/UTF8")
        tracks += ebml("ae", sub_track)
        cluster0 = ebml(
            "1f43b675",
            ebml("e7", b"\0") + ebml("a3", b"\x82\x00\x00\x80opening subtitle"),
        )
    else:
        hidden = vp9_frame(8, "420", 16, 16)
        cluster0 = ebml(
            "1f43b675",
            ebml("e7", b"\0") + ebml("a3", b"\x81\x00\x00\x88" + hidden),
        )
    cluster1_blocks = ebml("e7", origin_ms.to_bytes(2, "big"))
    sizes = [[16, 16], [32, 16], [16, 32]]
    for i in range(frame_count):
        pic = vp9_frame(8, "420", *sizes[i % len(sizes)])
        rel = (i * step_ms).to_bytes(2, "big", signed=True)
        cluster1_blocks += ebml("a3", b"\x81" + rel + b"\x80" + pic)
    cluster1 = ebml("1f43b675", cluster1_blocks)
    return header + ebml(
        "18538067",
        ebml("1549a966", info) + ebml("1654ae6b", tracks) + cluster0 + cluster1,
    )


def av1_clip(preamble_mode, origin_ms, step_ms):
    av1_files = generate_av1_hdr()
    packets = [av1_files[f"{i}.packet"] for i in range(3)]
    record = record_from_seed((ROOT / (SEED + ".mp4")).read_bytes())
    header = ebml("1a45dfa3", ebml("4282", b"matroska") + ebml("4287", b"\4") + ebml("4285", b"\4"))
    declared_ms = origin_ms + len(packets) * step_ms
    info = (
        ebml("2ad7b1", (1_000_000).to_bytes(3, "big"))
        + ebml("4489", struct.pack(">d", float(declared_ms)))
    )
    video_track = (
        ebml("d7", b"\1")
        + ebml("73c5", b"\1")
        + ebml("83", b"\1")
        + ebml("86", b"V_AV1")
        + ebml("63a2", record)
        + ebml("23e383", (step_ms * 1_000_000).to_bytes(4, "big"))
        + ebml("e0", ebml("b0", b"\x80") + ebml("ba", b"\x80"))
    )
    tracks = ebml("ae", video_track)
    if preamble_mode == "subtitle":
        sub_track = ebml("d7", b"\2") + ebml("73c5", b"\2") + ebml("83", b"\x11") + ebml("86", b"S_TEXT/UTF8")
        tracks += ebml("ae", sub_track)
        cluster0 = ebml(
            "1f43b675",
            ebml("e7", b"\0") + ebml("a3", b"\x82\x00\x00\x80opening subtitle"),
        )
    else:
        cluster0 = ebml(
            "1f43b675",
            ebml("e7", b"\0") + ebml("a3", b"\x81\x00\x00\x88" + packets[1]),
        )
    cluster1_blocks = ebml("e7", origin_ms.to_bytes(2, "big"))
    for i, pkt in enumerate(packets):
        rel = (i * step_ms).to_bytes(2, "big", signed=True)
        cluster1_blocks += ebml("a3", b"\x81" + rel + b"\x80" + pkt)
    cluster1 = ebml("1f43b675", cluster1_blocks)
    return header + ebml(
        "18538067",
        ebml("1549a966", info) + ebml("1654ae6b", tracks) + cluster0 + cluster1,
    )


def generate():
    files = {}
    cases = []
    for preamble in ["subtitle", "invisible"]:
        for origin_ms in [0, 60]:
            name = f"vp9-{preamble}-origin-{origin_ms}ms.mkv"
            files[name] = vp9_clip(preamble, origin_ms, 3, 20)
            cases.append(
                {
                    "file": name,
                    "codec": "VP9",
                    "preamble": preamble,
                    "origin_ns": origin_ms * 1_000_000,
                    "expected_duration_ns": 60_000_000,
                    "intervals": [[0, 20_000_000], [20_000_000, 40_000_000], [40_000_000, 60_000_000]],
                    "sizes": [[16, 16], [32, 16], [16, 32]],
                    "initial_cll": None,
                }
            )
    for preamble in ["subtitle", "invisible"]:
        for origin_ms in [0, 60]:
            name = f"av1-{preamble}-origin-{origin_ms}ms.mkv"
            files[name] = av1_clip(preamble, origin_ms, 20)
            cases.append(
                {
                    "file": name,
                    "codec": "AV1",
                    "preamble": preamble,
                    "origin_ns": origin_ms * 1_000_000,
                    "expected_duration_ns": 60_000_000,
                    "intervals": [[0, 20_000_000], [20_000_000, 40_000_000], [40_000_000, 60_000_000]],
                    "sizes": [[128, 128], [128, 128], [128, 128]],
                    "initial_cll": [1000.0, 400.0],
                }
            )
    files["manifest.json"] = (
        json.dumps(
            {
                "provenance": "Authored multi-cluster VP9 and AV1 Matroska clips with declared Duration where cluster 0 holds only subtitle or invisible pre-roll packets. No private media, FFmpeg or network.",
                "cases": cases,
                "sha256": {k: hashlib.sha256(v).hexdigest() for k, v in sorted(files.items())},
            },
            indent=2,
        )
        + "\n"
    ).encode()
    return files


def main():
    OUT.mkdir(parents=True, exist_ok=True)
    for name, data in generate().items():
        (OUT / name).write_bytes(data)


if __name__ == "__main__":
    main()
