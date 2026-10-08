#!/usr/bin/env python3
"""Original lossless camera tile lists and stock libaom generation-time oracle."""
import argparse
import hashlib
import json
import subprocess
import tempfile
from pathlib import Path

ORDER = [3, 0, 2, 1]
SUFFIXES = ["anchor.obu", "camera.obu", "header.obu", "list.obu", "list.yuv",
            "multi-list.obu", "multi-list.yuv"]


def authored(sb, depth, chroma, multi):
    output = bytearray()
    sx, sy = int(chroma != 444), int(chroma == 420)
    for plane in range(3):
        width = sb >> (sx if plane else 0)
        height = sb >> (sy if plane else 0)
        for y in range(height * 2):
            for x in range(width * 2):
                index = (y // height) * 2 + x // width
                source = ORDER[index]
                px = (source % 2) * width + x % width
                py = (source // 2) * height + y % height
                value = (71 + (3 * px + 5 * py + 23 * plane) % 96
                         + (9 * (index % 2) if multi else 0)) << (depth - 8)
                output.extend(value.to_bytes(1 if depth == 8 else 2, "little"))
    return output


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--generator", type=Path, required=True)
    args = parser.parse_args()
    root = Path(__file__).resolve().parents[1] / "tests/fixtures/playback-errors"
    for sb in [64, 128]:
        for depth in [8, 10, 12]:
            for chroma in [420, 422, 444]:
                tag = ("" if sb == 64 else "sb128-") if depth == 8 and chroma == 420 else f"d{depth}-c{chroma}-sb{sb}-"
                name_prefix = "av1-tile-list-" + tag
                with tempfile.TemporaryDirectory(prefix="fvid-tile-list-") as tmp:
                    prefix = Path(tmp) / "fixture"
                    subprocess.run([str(args.generator), str(prefix), str(sb), str(depth), str(chroma)], check=True)
                    for multi in [False, True]:
                        suffix = "multi-list.yuv" if multi else "list.yuv"
                        reference = Path(str(prefix) + "-" + suffix).read_bytes()
                        assert reference == authored(sb, depth, chroma, multi), (sb, depth, chroma, multi)
                    records = {}
                    for suffix in SUFFIXES:
                        data = Path(str(prefix) + "-" + suffix).read_bytes()
                        name = name_prefix + suffix
                        (root / name).write_bytes(data)
                        records[suffix] = {"file": name, "sha256": hashlib.sha256(data).hexdigest()}
                    manifest = {"size": [sb * 2, sb * 2], "tile_size": [sb, sb],
                                "superblock": sb, "depth": depth, "chroma": chroma,
                                "order": ORDER, "oracle": "stock libaom",
                                "multi_anchor_offsets": [0, 9 << (depth - 8)],
                                "multi_anchor_indices": [0, 1, 0, 1], "artifacts": records}
                    (root / (name_prefix + "generated.json")).write_text(json.dumps(manifest, indent=2) + "\n")


if __name__ == "__main__":
    main()
