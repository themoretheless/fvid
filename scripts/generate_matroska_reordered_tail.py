#!/usr/bin/env python3
"""Authored AVC, HEVC and PAFF Matroska clips with previous-explicit and unequal tail durations."""
import hashlib
import json
import struct
from pathlib import Path
from generate_audio_resample_window_fixture import ebml
from generate_avc_colour_formats import avc_configuration, make_pcm_packets
from generate_hevc_matroska_formats import unpack as unpack_hevc
from generate_avc_field_component_regressions import generate as generate_paff

ROOT = Path(__file__).resolve().parents[1] / "tests/fixtures/playback-errors"
OUT = ROOT / "matroska-reordered-tail"


def progressive_clip(codec, private, width, height, packets, times, durations, declared):
    header = ebml("1a45dfa3", ebml("4282", b"matroska") + ebml("4287", b"\4") + ebml("4285", b"\4"))
    info = ebml("2ad7b1", (1_000_000).to_bytes(3, "big"))
    if declared is not None:
        info += ebml("4489", struct.pack(">d", declared / 1_000_000))
    track = (
        ebml("d7", b"\1")
        + ebml("73c5", b"\1")
        + ebml("83", b"\1")
        + ebml("86", codec.encode("ascii"))
        + ebml("63a2", private)
        + ebml("e0", ebml("b0", width.to_bytes(2, "big")) + ebml("ba", height.to_bytes(2, "big")))
    )
    blocks = ebml("e7", b"\0")
    for index, (packet, time, duration) in enumerate(zip(packets, times, durations)):
        payload = b"\x81" + struct.pack(">h", time // 1_000_000)
        if duration is None:
            blocks += ebml("a3", payload + (b"\x80" if index == 0 else b"\0") + packet)
        else:
            group = ebml("a1", payload + b"\0" + packet) + ebml(
                "9b", (duration // 1_000_000).to_bytes(2, "big")
            )
            if index > 0:
                ref = (times[index - 1] - time) // 1_000_000
                group += ebml("fb", ref.to_bytes(1, "big", signed=True))
            blocks += ebml("a0", group)
    return header + ebml(
        "18538067",
        ebml("1549a966", info) + ebml("1654ae6b", ebml("ae", track)) + ebml("1f43b675", blocks),
    )


def paff_clip(config, packets, field_times, declared):
    header = ebml("1a45dfa3", ebml("4282", b"matroska") + ebml("4287", b"\4") + ebml("4285", b"\4"))
    info = ebml("2ad7b1", (1_000_000).to_bytes(3, "big"))
    if declared is not None:
        info += ebml("4489", struct.pack(">d", declared / 1_000_000))
    track = (
        ebml("d7", b"\1")
        + ebml("73c5", b"\1")
        + ebml("83", b"\1")
        + ebml("86", b"V_MPEG4/ISO/AVC")
        + ebml("63a2", config)
        + ebml("e0", ebml("b0", b"\x20") + ebml("ba", b"\x20"))
    )
    blocks = ebml("e7", b"\0")
    for index, (packet, time_ms) in enumerate(zip(packets, field_times)):
        group = ebml("a1", b"\x81" + struct.pack(">h", time_ms) + b"\0" + packet)
        refs = []
        if index >= 2:
            refs.append(field_times[index - 2] - time_ms)
        if index % 2 == 1:
            refs.append(field_times[index - 1] - time_ms)
        for ref in sorted(set(refs)):
            group += ebml("fb", ref.to_bytes(1, "big", signed=True))
        blocks += ebml("a0", group)
    return header + ebml(
        "18538067",
        ebml("1549a966", info) + ebml("1654ae6b", ebml("ae", track)) + ebml("1f43b675", blocks),
    )


def generate():
    files = {}
    progressive = []
    paff = []

    avc_cfg = avc_configuration(3, 8, 8, False)
    avc_pair, _ = make_pcm_packets(3, 8, 8)
    avc_packets = [avc_pair[0], avc_pair[1], avc_pair[1]]

    hevc_cfg, hevc_src, hevc_w, hevc_h, _ = unpack_hevc(ROOT / "hevc-pcm-reference-rext8.mp4")
    hevc_packets = [hevc_src[0], hevc_src[1], hevc_src[1]]

    for family, codec, cfg, width, height, packets in [
        ("avc", "V_MPEG4/ISO/AVC", avc_cfg, 32, 16, avc_packets),
        ("hevc", "V_MPEGH/ISO/HEVC", hevc_cfg, hevc_w, hevc_h, hevc_packets),
    ]:
        for name, times, durations, expected_ns in [
            ("previous-explicit", [0, 20_000_000, 40_000_000], [5_000_000, 5_000_000, None], 45_000_000),
            ("offset-previous-explicit", [70_000_000, 90_000_000, 110_000_000], [5_000_000, 5_000_000, None], 45_000_000),
        ]:
            origin = times[0]
            intervals = [
                [0, 5_000_000],
                [20_000_000, 25_000_000],
                [40_000_000, 45_000_000],
            ]
            inferred_file = f"{family}-{name}-inferred.mkv"
            declared_file = f"{family}-{name}-declared.mkv"
            files[inferred_file] = progressive_clip(codec, cfg, width, height, packets, times, durations, None)
            files[declared_file] = progressive_clip(
                codec, cfg, width, height, packets, times, durations, origin + expected_ns
            )
            progressive.append(
                {
                    "family": family,
                    "name": f"{family}-{name}",
                    "inferred": inferred_file,
                    "declared": declared_file,
                    "origin_ns": origin,
                    "expected_ns": expected_ns,
                    "size": [width, height],
                    "intervals": intervals,
                }
            )

    paff_sources = generate_paff()
    paff_manifest = json.loads(paff_sources["manifest.json"])
    for stem in ["y8-c8-top-pcm", "y8-c14-bottom-pcm"]:
        case = next(c for c in paff_manifest["cases"] if c["file"] == f"{stem}.mp4")
        cfg = paff_sources[case["configuration"]]
        packets = [paff_sources[p] for p in case["packets"]]
        for label, field_times, origin_ns, expected_ns, intervals in [
            (
                "unequal-declared-tail",
                [0, 7, 20, 27],
                0,
                40_000_000,
                [[0, 20_000_000], [20_000_000, 40_000_000]],
            ),
            (
                "offset-unequal-declared-tail",
                [50, 57, 70, 77],
                50_000_000,
                40_000_000,
                [[0, 20_000_000], [20_000_000, 40_000_000]],
            ),
        ]:
            filename = f"paff-{stem}-{label}.mkv"
            files[filename] = paff_clip(cfg, packets, field_times, origin_ns + expected_ns)
            paff.append(
                {
                    "file": filename,
                    "stem": stem,
                    "origin_ns": origin_ns,
                    "expected_ns": expected_ns,
                    "intervals": intervals,
                }
            )

    files["manifest.json"] = (
        json.dumps(
            {
                "provenance": "Authored short AVC, HEVC and PAFF Matroska clips without TrackEntry.DefaultDuration testing previous-explicit and declared PAFF tail durations. No private media, FFmpeg or network.",
                "progressive": progressive,
                "paff": paff,
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
