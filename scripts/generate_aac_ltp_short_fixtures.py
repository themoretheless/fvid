#!/usr/bin/env python3
"""Own short-window (EightShort) AAC LTP fixtures for LtpHistory, LtpAnalysis, and LtpChannel."""
import json
import math
import struct
from generate_aac_ssr_fixtures import windows
from generate_he_aac_packet_fixtures import DEST, field, packed

GAINS = [.570829, .696616, .813004, .911304, .984900, 1.067894, 1.194601, 1.369533]
OFFSET = 8


def f32(x):
    return struct.unpack('<f', struct.pack('<f', x))[0]


class ShortOracle:
    def __init__(self, n):
        self.n = n
        self.previous = 0
        self.overlap = [0.] * n
        self.history = [0.] * (4 * n)
        self.kbd = {n: windows(n, 4), n // 8: windows(n // 8, 6)}
        self.cos = {
            size: [
                [math.cos(math.pi / size * (i + .5 + size / 2) * (k + .5)) for i in range(2 * size)]
                for k in range(size if size <= 128 else 8)
            ]
            for size in (n, n // 8)
        }

    def weight(self, size, i, shape):
        return self.kbd[size][i] if shape else math.sin(math.pi * (i + .5) / (2 * size))

    def long_weight(self, i, seq, shape):
        n = self.n
        small = n // 8
        flat = (n - small) // 2
        if seq == 1 and i >= n:
            j = i - n
            return 1. if j < flat else self.weight(small, small + j - flat, shape) if j < flat + small else 0.
        if seq == 3 and i < n:
            return 0. if i < flat else self.weight(small, i - flat, self.previous) if i < flat + small else 1.
        return self.weight(n, i, self.previous if i < n else shape)

    def run(self, seq, shape, q, active, lag, coefficient, used, short_windows=None, tns=None):
        n = self.n
        small = n // 8
        flat = (n - small) // 2
        spectra = [
            [v * 1024. for v in row] + ([0.] * (small - len(row)) if seq == 2 else [])
            for row in q
        ]
        if active:
            if seq == 2:
                gain = GAINS[coefficient]
                for w, pred in enumerate(short_windows):
                    if pred is None:
                        continue
                    win_lag = lag if pred['lag'] is None else lag + int(pred['lag']) - OFFSET
                    start = 2 * n - win_lag
                    est = [
                        v * gain * self.weight(small, i, self.previous if w == 0 and i < small else shape)
                        for i, v in enumerate(self.history[start:start + 2 * small])
                    ]
                    predicted = [sum(v * c for v, c in zip(est, self.cos[small][k])) for k in range(small)]
                    if tns is not None and tns[w] is not None:
                        reverse, lpc = tns[w]
                        hist = 0.
                        for k in (range(7, -1, -1) if reverse else range(8)):
                            orig = predicted[k]
                            predicted[k] = orig + lpc * hist
                            hist = orig
                    for k in range(small):
                        spectra[w][k] = f32(spectra[w][k] + predicted[k])
            else:
                estimate = [
                    v * GAINS[coefficient] * self.long_weight(i, seq, shape)
                    for i, v in enumerate(self.history[2 * n - lag:4 * n - lag])
                ]
                prediction = [sum(v * c for v, c in zip(estimate, self.cos[n][k])) for k in range(8)]
                spectra[0] = [f32(v + prediction[k]) if used[k // 4] else v for k, v in enumerate(spectra[0])]
        if seq == 2 and tns is not None:
            for w in range(8):
                if tns[w] is None:
                    continue
                reverse, lpc = tns[w]
                hist = 0.
                for k in (range(7, -1, -1) if reverse else range(8)):
                    val = spectra[w][k] - lpc * hist
                    spectra[w][k] = f32(val)
                    hist = val
        block = [0.] * (2 * n)
        if seq == 2:
            for w, spectrum in enumerate(spectra):
                for i in range(2 * small):
                    value = 2 / small * sum(v * self.cos[small][k][i] for k, v in enumerate(spectrum))
                    block[flat + w * small + i] += value * self.weight(
                        small, i, self.previous if w == 0 and i < small else shape
                    )
        else:
            block = [
                2 / n * sum(v * self.cos[n][k][i] for k, v in enumerate(spectra[0])) * self.long_weight(i, seq, shape)
                for i in range(2 * n)
            ]
        raw = [a + b for a, b in zip(self.overlap, block[:n])]
        self.overlap = block[n:]
        self.history = self.history[n:2 * n] + raw + self.overlap + [0.] * n
        self.previous = shape
        return [f32(v / 65536) for v in raw]


def encode_short_ltp(lag, coefficient, short_windows):
    bits = field(lag, 11) + field(coefficient, 3)
    for w in short_windows:
        if w is None:
            bits += '0'
        else:
            bits += '1'
            if w['lag'] is None:
                bits += '0'
            else:
                bits += '1' + field(w['lag'], 4)
    return bits


def main():
    for stale in DEST.glob('aac-ltp-short-aot*'):
        stale.unlink()
    blob = bytearray()
    gold = bytearray()
    inactive_gold = bytearray()
    cases = []
    sequences = [0, 1, 2, 2, 3, 0]
    for n in (960, 1024):
        for with_tns in (False, True):
            oracle = ShortOracle(n)
            control = ShortOracle(n)
            rows = []
            start = len(gold)
            for frame, seq in enumerate(sequences):
                shape = frame % 2
                active = frame >= 2
                lag = n - 16 * (frame % 3)
                coef = (frame * 3 + 1) % 8
                used = [True, frame % 2 == 0]
                short_windows = [
                    (
                        None
                        if (w + frame) % 3 == 2
                        else dict(lag=(w * 2 + frame) % 16 if (w + frame) % 2 == 0 else None)
                    )
                    for w in range(8)
                ]
                q = [
                    [(-1 if (frame + w + k) % 3 == 0 else 1 if (frame + w + k) % 3 == 1 else 0) for k in range(8)]
                    for w in range(8 if seq == 2 else 1)
                ]
                tns_spec = (
                    [(bool((w + frame) % 2), 0.25 if w % 2 == 0 else -0.125) if w % 3 != 1 else None for w in range(8)]
                    if (seq == 2 and with_tns)
                    else None
                )
                ltp_offset = len(blob)
                ltp_bytes = 0
                if active and seq == 2:
                    raw = packed('101' + encode_short_ltp(lag, coef, short_windows))
                    ltp_bytes = len(raw)
                    blob.extend(raw)
                pcm = oracle.run(seq, shape, q, active, lag, coef, used, short_windows, tns=tns_spec)
                ctrl = control.run(seq, shape, q, active and seq != 2, lag, coef, used, None, tns=tns_spec)
                rows.append(dict(
                    sequence=seq,
                    shape=shape,
                    active=active,
                    lag=lag,
                    coefficient=coef,
                    used=used,
                    short_windows=short_windows if seq == 2 else None,
                    ltp_offset=ltp_offset,
                    ltp_bytes=ltp_bytes,
                    q=q,
                    tns=tns_spec,
                    reference_offset=len(gold),
                ))
                gold.extend(struct.pack('<' + str(n) + 'f', *pcm))
                inactive_gold.extend(struct.pack('<' + str(n) + 'f', *ctrl))
            cases.append(dict(
                name=f'n{n}-tns{int(with_tns)}',
                n=n,
                with_tns=with_tns,
                frames=rows,
                reference_offset=start,
                reference_bytes=len(gold) - start,
            ))
    (DEST / 'aac-ltp-short-packets.bin').write_bytes(blob)
    (DEST / 'aac-ltp-short-reference.f32le').write_bytes(gold)
    (DEST / 'aac-ltp-short-inactive-reference.f32le').write_bytes(inactive_gold)
    (DEST / 'aac-ltp-short.json').write_text(json.dumps(dict(
        cases=cases,
        provenance='Own EightShort AAC LTP syntax, history, analysis, TNS and LtpChannel trajectories for 960 and 1024 samples with per-subwindow lag offsets and sine/KBD shapes. No external codecs or private media.',
    ), indent=2) + '\n')
    print(f'generated {len(cases)} short-window LTP channel cases')


if __name__ == '__main__':
    main()
