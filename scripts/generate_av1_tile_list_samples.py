#!/usr/bin/env python3
"""Original lossless camera tile lists and stock libaom generation-time oracle."""
import argparse
import hashlib
import json
import itertools
import subprocess
import tempfile
from pathlib import Path

ORDER = [3, 0, 2, 1]
SUFFIXES = ["anchor.obu", "camera.obu", "header.obu", "list.obu", "list.yuv",
            "multi-list.obu", "multi-list.yuv"]


def authored(sb, depth, chroma, multi, q=0, motion=0):
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
                if motion:
                    px = min(width * 2 - 1, px + (4 >> (sx if plane else 0)))
                    py = min(height * 2 - 1, py + (2 >> (sy if plane else 0)))
                value = ((64 if motion else 71) + (3 * px + 5 * py + 23 * plane) % 96
                         + ((px // 8 + py // 8 + plane) % 7 - 3 if q else 0)
                         + (9 * (index % 2) if multi else 0)) << (depth - 8)
                output.extend(value.to_bytes(1 if depth == 8 else 2, "little"))
    return output


def assemble_tiles(prefix, sb, depth, chroma):
    tiles = [Path(str(prefix) + f"-tile{i}.yuv").read_bytes() for i in range(4)]
    output = bytearray()
    offset = 0
    bps = 1 if depth == 8 else 2
    for plane in range(3):
        width = sb >> (int(chroma != 444) if plane else 0)
        height = sb >> (int(chroma == 420) if plane else 0)
        for y in range(height * 2):
            for column in range(2):
                tile = tiles[y // height * 2 + column]
                start = offset + (y % height) * width * bps
                output.extend(tile[start:start + width * bps])
        offset += width * height * bps
    assert all(len(tile) == offset for tile in tiles)
    return output


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--generator", type=Path, required=True)
    parser.add_argument("--motion", type=int, choices=[0, 1], help="generate only this motion family")
    args = parser.parse_args()
    root = Path(__file__).resolve().parents[1] / "tests/fixtures/playback-errors"
    for motion, adapted, q, sb, depth, chroma in itertools.product([0, 1] if args.motion is None else [args.motion], [0, 1, 2], [0, 32], [64, 128], [8, 10, 12], [420, 422, 444]):
        tag = ("" if sb == 64 else "sb128-") if depth == 8 and chroma == 420 else f"d{depth}-c{chroma}-sb{sb}-"
        if q:
            tag = f"q{q}-d{depth}-c{chroma}-sb{sb}-"
        if adapted:
            tag = ("none-" if adapted == 2 else "cdf-") + tag
        if motion:
            tag = "mv-" + tag
        name_prefix = "av1-tile-list-" + tag
        with tempfile.TemporaryDirectory(prefix="fvid-tile-list-") as tmp:
            prefix = Path(tmp) / "fixture"
            subprocess.run([str(args.generator), str(prefix), str(sb), str(depth), str(chroma), str(q), str(adapted), str(motion)], check=True)
            single = Path(str(prefix) + "-list.yuv").read_bytes()
            multiple = Path(str(prefix) + "-multi-list.yuv").read_bytes()
            assert single == assemble_tiles(prefix, sb, depth, chroma), "tile-list must match separate camera-tile reconstruction"
            if q:
                assert single != authored(sb, depth, chroma, False, q, motion), "lossy output must differ from source"
                assert multiple != single, "external anchor selection must affect output"
            else:
                assert single == authored(sb, depth, chroma, False, motion=motion)
                if motion:
                    assert multiple != single
                else:
                    assert multiple == authored(sb, depth, chroma, True)
            records = {}
            for suffix in SUFFIXES:
                data = Path(str(prefix) + "-" + suffix).read_bytes()
                name = name_prefix + suffix
                (root / name).write_bytes(data)
                records[suffix] = {"file": name, "sha256": hashlib.sha256(data).hexdigest()}
            manifest = {"size": [sb * 2, sb * 2], "tile_size": [sb, sb],
                        "superblock": sb, "depth": depth, "chroma": chroma, "quantizer": q, "adapted_anchor_cdf": bool(adapted), "primary_ref_none": adapted == 2,
                        "order": ORDER, "oracle": "stock libaom", "motion_shift": [4, 2] if motion else [0, 0], "cpu_used": 0 if motion else 6,
                        "multi_anchor_offsets": [0, 9 << (depth - 8)],
                        "multi_anchor_indices": [0, 1, 0, 1], "artifacts": records}
            (root / (name_prefix + "generated.json")).write_text(json.dumps(manifest, indent=2) + "\n")


if __name__ == "__main__":
    main()
