#!/usr/bin/env python3
"""Owned full-chroma AV1 moving inter patterns and resized references; generation only."""
import argparse
import hashlib
import itertools
import json
import subprocess
import tempfile
from pathlib import Path
from generate_av1_show_existing_samples import webm


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--encoder', type=Path, required=True)
    parser.add_argument('--oracle', type=Path, required=True)
    parser.add_argument('--second-oracle', type=Path, required=True)
    parser.add_argument('--depth', type=int, choices=[8, 10, 12])
    parser.add_argument('--layout', choices=['422', '444'])
    args = parser.parse_args()
    root = Path(__file__).resolve().parents[1] / 'tests/fixtures/playback-errors'
    records = []
    for depth, layout, resized, pattern in itertools.product(
        [args.depth] if args.depth else [8, 10, 12],
        [args.layout] if args.layout else ['422', '444'], [False, True], [0, 1]
    ):
        w, h, count = 191, 127, 8
        sx, sy = (1, 0) if layout == '422' else (0, 0)
        def plane(width, height, channel, t):
            values = []
            for y in range(height):
                for x in range(width):
                    xx, yy = x + t * 2, y + t
                    value = 50 + (xx * 3 + yy * 5 + (xx * yy % 19) * 2 + channel * 11) % 150
                    if (x - t * 3) % width < width // 3 and height // 4 < y < height * 3 // 4:
                        value = 75 + ((x - t * 3) * 7 + (y + t * 2) * 13 + channel * 17) % 140
                    if pattern:
                        value = max(0, min(255, value + ((t % 3) - 1) * 12))
                    values.append(value)
            return bytes(values) if depth == 8 else b''.join((v << (depth - 8)).to_bytes(2, 'little') for v in values)
        source = [plane(w, h, 0, t) + plane((w + (1 << sx) - 1) >> sx, h, 1, t) + plane((w + (1 << sx) - 1) >> sx, h, 2, t) for t in range(count)]
        name = f'av1-chroma-inter-layout{layout}-depth{depth}-resize{int(resized)}-pattern{pattern}'
        coded, golden, wrapped = [root / (name + suffix) for suffix in ['.obu', '.yuv', '.webm']]
        sizes = [[(w * 8 + 8) // 16, (h * 8 + 8) // 16] if resized and t == 0 else [w, h] for t in range(count)]
        with tempfile.TemporaryDirectory(prefix='fvid-owned-chroma-inter-') as tmp:
            input_path = Path(tmp) / 'owned.y4m'
            input_path.write_bytes(f'YUV4MPEG2 W{w} H{h} F50:1 Ip A1:1 C{layout if depth == 8 else layout + "p" + str(depth)}\nFRAME\n'.encode() + b'FRAME\n'.join(source))
            subprocess.run([
                str(args.encoder), '--obu', '--cpu-used=0', '--passes=1', f'--limit={count}',
                '--sb-size=64', '--lag-in-frames=0', '--auto-alt-ref=0', '--kf-min-dist=99', '--kf-max-dist=99',
                *([f'--profile={1 if layout == "444" else 2}'] if depth < 12 else []),
                '--end-usage=q', '--cq-level=40', '--deltaq-mode=0', '--enable-qm=0',
                f'--resize-mode={int(resized)}', '--resize-denominator=8', '--resize-kf-denominator=16',
                f'--bit-depth={depth}', f'--input-bit-depth={depth}',
                '--min-partition-size=4', '--max-partition-size=64', '--enable-rect-partitions=1',
                '--enable-ab-partitions=0', '--enable-1to4-partitions=0', '--enable-palette=0', '--enable-intrabc=0',
                '--enable-cdef=0', '--enable-restoration=0', '--enable-ref-frame-mvs=0',
                '--enable-obmc=1', '--enable-warped-motion=1', '--enable-masked-comp=1',
                '--enable-interintra-comp=1', f'--output={coded}', str(input_path)
            ], check=True)
            subprocess.run([str(args.oracle), '--i420', '--rawvideo', f'--output-bit-depth={depth}', f'--output={golden}', str(coded)], check=True)
            cross = Path(tmp) / 'cross.yuv'
            subprocess.run([str(args.second_oracle), '--demuxer', 'section5', '--muxer', 'yuv', '-i', str(coded), '-o', str(cross)], check=True)
            pixels = golden.read_bytes()
            assert cross.read_bytes() == pixels, name + ': independent oracle mismatch'
            expected_len = sum((x * y + 2 * ((x + (1 << sx) - 1) >> sx) * y) * (1 if depth == 8 else 2) for x, y in sizes)
            assert len(pixels) == expected_len, (name, len(pixels), expected_len)
        data = coded.read_bytes()
        wrapped.write_bytes(webm(data, (w, h)))
        records.append(dict(file=coded.name, reference=golden.name, webm=wrapped.name,
            sha256=hashlib.sha256(data).hexdigest(), reference_sha256=hashlib.sha256(pixels).hexdigest(),
            webm_sha256=hashlib.sha256(wrapped.read_bytes()).hexdigest(), source_sha256=hashlib.sha256(b''.join(source)).hexdigest(),
            layout=layout, depth=depth, subsampling=[sx, sy], frames=count, sizes=sizes, resized=resized, pattern=pattern,
            oracles=['libaom', 'dav1d']))
    (root / 'av1-chroma-inter-generated.json').write_text(json.dumps(dict(fixtures=records), indent=2) + '\n')


if __name__ == '__main__':
    main()
