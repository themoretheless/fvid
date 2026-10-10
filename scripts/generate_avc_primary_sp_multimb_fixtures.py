#!/usr/bin/env python3
"""Original 32x32 SP skip streams with cross-macroblock and cross-slice edges."""
import argparse
import json
import subprocess
import tempfile
from pathlib import Path
from generate_avc_mbaff_direct_samples import Writer
from avc_fixture_mp4 import mux, annexb

DEST = Path(__file__).resolve().parents[1] / 'tests/fixtures/playback-errors'


def configuration():
    b = Writer()
    b.u(88, 8); b.u(0, 8); b.u(10, 8)
    for v in [0, 0, 0, 0, 1]: b.ue(v)
    b.u(0); b.ue(1); b.ue(1); b.u(1); b.u(1); b.u(0); b.u(0)
    sps = b.nal(0x67)
    b = Writer()
    b.ue(0); b.ue(0); b.u(0); b.u(0)
    for _ in range(3): b.ue(0)
    b.u(0); b.u(0, 2)
    for _ in range(3): b.se(0)
    b.u(1); b.u(0); b.u(0)
    pps = b.nal(0x68)
    return bytes([1, 88, 0, 10, 255, 225]) + len(sps).to_bytes(2, 'big') + sps + bytes([1]) + len(pps).to_bytes(2, 'big') + pps


def packet(nals):
    return b''.join(len(n).to_bytes(4, 'big') + n for n in nals)


def source():
    return [[base + sign * (y // 4 * 2 + x // 4 * 3) for y in range(size) for x in range(size)] for size, base, sign in [(32, 64, 1), (16, 96, 1), (16, 160, -1)]]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--jm-decoder', type=Path)
    args = parser.parse_args()
    config = configuration()
    planes = source()
    b = Writer()
    b.ue(0); b.ue(2); b.ue(0); b.u(0, 4); b.ue(0); b.u(0, 4)
    b.u(0); b.u(0); b.se(0); b.ue(1)
    for mb in range(4):
        b.ue(25); b.align()
        for plane, size, block in zip(planes, [32, 16, 16], [16, 8, 8]):
            for y in range(block):
                for x in range(block): b.u(plane[(mb // 2 * block + y) * size + mb % 2 * block + x], 8)
    first = packet([b.nal(0x65)])
    cases = []
    for slices, motion in [(1, False), (4, False), (4, True)]:
        for mode in [0, 1, 2]:
            frames = [(0, True, first)]
            packets = [first.hex()]
            for i, qs in enumerate([0, 26, 51], 1):
                nals = []
                for start in (range(4) if slices == 4 else [0]):
                    b = Writer()
                    b.ue(start); b.ue(3); b.ue(0); b.u(i, 4); b.u(2 * i, 4)
                    b.u(0); b.u(0); b.u(0); b.se(24); b.u(0); b.se(qs - 26); b.ue(mode)
                    if mode != 1: b.se(0); b.se(0)
                    if motion:
                        # Each MB is a slice: spatial MV predictor is unavailable/zero.
                        dx, dy = [(1, -1), (3, 2), (-5, 7)][i - 1]
                        b.ue(0); b.ue(0); b.se(dx); b.se(dy); b.ue(0)
                    else:
                        b.ue(1 if slices == 4 else 4)
                    nals.append(b.nal(0x41))
                p = packet(nals)
                packets.append(p.hex()); frames.append((i, False, p))
            name = f'avc-primary-sp-multimb-s{slices}-filter{mode}' + ('-motion' if motion else '')
            filename = name + '-synthetic.mp4'
            reference = name + '-reference.yuv'
            (DEST / filename).write_bytes(mux(config, frames, 32, 32, 30))
            if args.jm_decoder:
                with tempfile.TemporaryDirectory(prefix='fvid-sp-multimb-') as tmp:
                    d = Path(tmp)
                    (d / 'decoder.cfg').write_text('')
                    (d / 'owned.264').write_bytes(annexb(config, frames))
                    with (d / 'jm.log').open('w') as log:
                        subprocess.run([str(args.jm_decoder), '-d', str(d / 'decoder.cfg'), '-p', f'InputFile={d}/owned.264', '-p', f'OutputFile={d}/owned.yuv', '-p', 'FileFormat=0', '-p', 'RefFile=nonexistent.yuv'], cwd=d, stdout=log, stderr=subprocess.STDOUT, check=True)
                    raw = (d / 'owned.yuv').read_bytes()
                    assert len(raw) == 6144, (name, len(raw))
                    (DEST / reference).write_bytes(raw)
            cases.append(dict(file=filename, reference=reference, configuration=config.hex(), packets=packets, slices=slices, mode=mode, motion=motion))
    (DEST / 'avc-primary-sp-multimb.json').write_text(json.dumps(dict(cases=cases, provenance='Original smooth planar steps, 32x32 four I_PCM macroblocks then three primary SP skip pictures at QPY50 and QSY0/26/51. One or four slices; deblocking modes0/1/2. Additional four-slice coded inter pictures use quarter-sample MV (1,-1), (3,2), (-5,7) and zero residual. Optional explicit JM reference capture; normal generation and tests offline without FFmpeg. No private media.'), indent=2) + '\n')


if __name__ == '__main__': main()
