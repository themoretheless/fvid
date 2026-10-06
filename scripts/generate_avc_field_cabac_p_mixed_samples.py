#!/usr/bin/env python3
"""Owned single-slice CABAC P fields mixing intra and skip/coded blocks."""
import argparse, tempfile, subprocess, hashlib, json
from pathlib import Path
from generate_avc_field_cabac_samples import configuration, field
from generate_avc_mbaff_direct_samples import Writer, CabacWriter
from avc_fixture_mp4 import mux, annexb


def prediction(bottom, index, placement, intra, inter, deblock, init):
    b = Writer()
    b.ue(0); b.ue(0); b.ue(0); b.u(1, 4); b.u(1); b.u(int(bottom)); b.u(index, 4)
    b.u(1); b.ue(0); b.u(0); b.u(0); b.ue(init); b.se(24); b.ue(deblock)
    if deblock != 1: b.se(6); b.se(6)
    while len(b.bits) % 8: b.u(1)
    c = CabacWriter(init, 50)
    intra_at = 0 if placement == 'intra-first' else 1
    for address in range(2):
        is_intra = address == intra_at
        c.decision(11 + int(address == 1 and (intra_at == 0 or inter == 'coded')), int(not is_intra and inter == 'skip'))
        if is_intra:
            c.decision(14, 1)
            c.decision(17, int(intra != 'i4-zero'))
            if intra != 'i4-zero':
                c.range -= 2; c.renormalize()  # PCM termination bin is zero.
                c.decision(18, 0); c.decision(19, 0)
                c.decision(20, 1); c.decision(20, 0)  # I16 DC / CBP0.
            else:
                for _ in range(16): c.decision(68, 1)  # predicted DC mode.
            c.decision(64, 0)
            if intra == 'i4-zero':
                for ctx in ([73, 74, 75, 76] if address == 0 else [74, 74, 76, 76]): c.decision(ctx, 0)
                c.decision(77, 0)
            else:
                c.decision(60, 0)
                c.decision(88 if address == 0 else 87, 1)
                c.decision(277, 0); c.decision(278, 1); c.decision(339, 1)
                c.decision(228, 0); c.bypass(int(intra == 'i16-negative'))
        elif inter == 'coded':
            c.decision(14, 0); c.decision(15, 0); c.decision(16, 0)
            c.mvd(0, 1, 0); c.mvd(1, -1, 0)
            for ctx in ([73, 74, 75, 76] if address == 0 else [74, 74, 76, 76]): c.decision(ctx, 0)
            c.decision(77, 0)
        if address == 0:
            c.range -= 2; c.renormalize()  # end_of_slice_flag=0.
    b.bits.extend(c.finish())
    return b.nal(0x41, trailing=False)


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--jm-decoder', type=Path, required=True)
    p.add_argument('--constrained', action='store_true', help='Forbid intra prediction from inter neighbours; write separate paired fixtures')
    p.add_argument('--prediction-control', action='store_true', help='Unconstrained control with biased reference fields')
    args = p.parse_args()
    assert not (args.constrained and args.prediction_control)
    out = Path(__file__).resolve().parents[1] / 'tests/fixtures/playback-errors'
    records = []
    with tempfile.TemporaryDirectory(prefix='fvid-field-cabac-p-mixed-') as tmp:
        d = Path(tmp); cfg = d / 'decoder.cfg'; cfg.write_text('')
        for depth in [8, 10]:
            for reverse in [False, True]:
                for placement in ['intra-first', 'intra-last']:
                    for intra in ['i4-zero', 'i16-positive', 'i16-negative']:
                        for inter in ['skip', 'coded']:
                            for deblock in [0, 1, 2]:
                                for init in range(3):
                                    order = [True, False] if reverse else [False, True]
                                    frames = []
                                    for index, bottom in enumerate(order):
                                        nals = [field(bottom, index, address, 'positive' if address == 0 else 'negative', deblock, biased=args.constrained or args.prediction_control) for address in range(2)]
                                        frames.append((index, index == 0, b''.join(len(n).to_bytes(4, 'big') + n for n in nals)))
                                    for n, bottom in enumerate(order):
                                        nal = prediction(bottom, 2 + n, placement, intra, inter, deblock, init)
                                        frames.append((2 + n, False, len(nal).to_bytes(4, 'big') + nal))
                                    config = configuration(depth, constrained=args.constrained)
                                    prefix = 'avc-field-cabac-p-mixed-constrained-' if args.constrained else 'avc-field-cabac-p-mixed-prediction-control-' if args.prediction_control else 'avc-field-cabac-p-mixed-'
                                    name = f'{prefix}{depth}bit-'+('bottom-first' if reverse else 'top-first')+f'-{placement}-{intra}-{inter}-init{init}-filter{deblock}'
                                    coded = d / (name + '.264'); oracle = d / (name + '.yuv')
                                    coded.write_bytes(annexb(config, frames))
                                    subprocess.run([str(args.jm_decoder), '-d', str(cfg), '-p', f'InputFile={coded}', '-p', f'OutputFile={oracle}', '-p', 'FileFormat=0', '-p', 'RefFile=nonexistent.yuv'], cwd=d, check=True, stdout=subprocess.DEVNULL)
                                    pixels = oracle.read_bytes()
                                    assert len(pixels) == 3072 * (2 if depth == 10 else 1), (name, len(pixels))
                                    data = mux(config, frames, 32, 32, 50)
                                    (out / (name + '.mp4')).write_bytes(data); (out / (name + '.yuv')).write_bytes(pixels)
                                    records.append(dict(file=name+'.mp4', sha256=hashlib.sha256(data).hexdigest(), oracle_sha256=hashlib.sha256(pixels).hexdigest()))
    (out / ('avc-field-cabac-p-mixed-constrained-generated.json' if args.constrained else 'avc-field-cabac-p-mixed-prediction-control-generated.json' if args.prediction_control else 'avc-field-cabac-p-mixed-generated.json')).write_text(json.dumps(dict(generator='owned single-slice CABAC mixed intra/P field writer', fixtures=records), indent=2)+'\n')

if __name__ == '__main__': main()
