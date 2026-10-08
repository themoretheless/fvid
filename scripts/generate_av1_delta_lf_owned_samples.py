#!/usr/bin/env python3
"""Owned AV1 scalar/multi delta-LF escape and clipping cases; offline test goldens."""
import argparse, hashlib, itertools, json, subprocess, tempfile
from pathlib import Path
from generate_av1_mixed_lossless_samples import encode, key
from generate_av1_show_existing_samples import sequence, show, webm


def main():
    p = argparse.ArgumentParser(description=__doc__)
    for arg in ['writer', 'oracle', 'second-oracle']:
        p.add_argument('--' + arg, type=Path, required=True)
    a = p.parse_args()
    root = Path(__file__).resolve().parents[1] / 'tests/fixtures/playback-errors'
    records = []
    for resolution, multi, adaptive in itertools.product(range(4), [False, True], [False, True]):
        for values in ([0], [1], [-2], [3], [-63], [255]) if not multi else ([1, -2, 3, -63], [-255, 255, -3, 2]):
            delta = (resolution, multi, values)
            entropy, _ = encode(a.writer, 64, 5, False, adaptive, 8, residual_everywhere=True, delta_lf=delta)
            data = sequence(False, False, False) + key(entropy, 64, False, adaptive, delta_lf=delta) + show(False, False, False, 0, 0)
            name = f'av1-delta-lf-owned-r{resolution}-multi{int(multi)}-adapt{int(adaptive)}-v' + '_'.join(map(str, values))
            file, reference, wrapped = name + '.obu', name + '.yuv', name + '.webm'
            (root / file).write_bytes(data)
            subprocess.run([str(a.oracle), str(root / file), '1', str(root / reference)], check=True)
            golden = (root / reference).read_bytes()
            assert len(golden) == 1536
            with tempfile.TemporaryDirectory(prefix='fvid-delta-lf-reference-') as tmp:
                cross = Path(tmp) / 'cross.yuv'
                subprocess.run([str(a.second_oracle), str(root / file), str(cross)], check=True)
                assert cross.read_bytes() == golden, name
            container = webm(data)
            (root / wrapped).write_bytes(container)
            records.append(dict(file=file, sha256=hashlib.sha256(data).hexdigest(), reference=reference,
                                reference_sha256=hashlib.sha256(golden).hexdigest(), webm=wrapped,
                                webm_sha256=hashlib.sha256(container).hexdigest(), resolution=resolution,
                                multi=multi, adaptive=adaptive, values=values))
    (root / 'av1-delta-lf-owned-generated.json').write_text(json.dumps(dict(fixtures=records), indent=2) + '\n')


if __name__ == '__main__':
    main()
