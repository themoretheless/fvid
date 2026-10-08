#!/usr/bin/env python3
"""Original SBR rate/header geometry oracles; explicit generation only.

ISO/IEC 14496-3:2001/Amd.1:2003, 4.6.18.3.2.1 and 4.6.18.3.6.
The original published equations take precedence over these GOST translation
errors: startMin/stopMin use <32000, [32000,64000), >=64000; stopDk uses
(p+1)/13, not (p-1)/13; stop index 14 takes 2*k0, not the sum branch.

Every start/stop pair at every mapped internal rate is checked, plus composed
master/high/low/noise geometry. This is numerical protocol coverage, not an
encoded HE-AAC playback fixture or acceptance claim. No private media, codec,
FFmpeg, libav, network or test-time generation is used.
"""
from decimal import Decimal as D, localcontext
from pathlib import Path
import json
from generate_aac_sbr_frequency_oracles import nint, tables

RATES = [16000, 22050, 24000, 32000, 44100, 48000, 64000,
         88200, 96000, 128000, 176400, 192000]
# Normative numeric table, independently transcribed from the publication.
OFFSETS = [
    list(range(-8, 8)),
    [-5,-4,-3,-2,-1,0,1,2,3,4,5,6,7,9,11,13],
    [-5,-3,-2,-1,0,1,2,3,4,5,6,7,9,11,13,16],
    [-6,-4,-2,-1,0,1,2,3,4,5,6,7,9,11,13,16],
    [-4,-2,-1,0,1,2,3,4,5,6,7,9,11,13,16,20],
    [-2,-1,0,1,2,3,4,5,6,7,9,11,13,16,20,24],
]

def boundaries(rate):
    row = {16000:0, 22050:1, 24000:2, 32000:3}.get(rate, 4 if rate <= 64000 else 5)
    base = D(3000 if rate < 32000 else 4000 if rate < 64000 else 5000)
    start_min = nint(base * 128 / rate)
    stop_min = nint(base * 256 / rate)
    # Evaluate all 14 endpoints independently at 80 digits. Rust evaluates
    # only interior endpoints and takes its last endpoint as the exact 64.
    endpoints = [nint(D(stop_min) * (D(64)/stop_min)**(D(p)/13)) for p in range(14)]
    widths = sorted(endpoints[p+1]-endpoints[p] for p in range(13))
    stops = [min(64, stop_min + sum(widths[:p])) for p in range(14)]
    return [start_min + d for d in OFFSETS[row]], stops

def main():
    vectors = []
    with localcontext() as ctx:
        ctx.prec = 80
        for rate in RATES:
            starts, stops = boundaries(rate)
            limit = 48 if rate <= 32000 else 35 if rate == 44100 else 32
            for start in range(16):
                for stop in range(16):
                    k0 = starts[start]
                    k2 = stops[stop] if stop < 14 else min(64, k0 * (stop-12))
                    bounds = [k0,k2] if 0 < k0 <= 32 and k0 < k2 <= 64 and k2-k0 <= limit else None
                    # Exercise all scale/alter, crossover and noise fields across
                    # the full grid, with extra cases on each legal boundary.
                    controls = [
                        [(start+stop)%4, bool(start%2), (start+3*stop)%8, stop%4],
                        [0, False, 0, 0],
                        [2, True, 0, 2],
                    ]
                    expected = []
                    for scale, alter, cross, noise in controls:
                        try:
                            if bounds is None: raise ValueError()
                            value = tables(k0,k2,scale,alter,cross,noise)
                        except ValueError:
                            value = None
                        expected.append(value)
                    vectors.append({'rate':rate, 'start':start, 'stop':stop,
                                    'bounds':bounds, 'controls':controls, 'tables':expected})
    destination = Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/aac-sbr-rate-oracles.json'
    destination.write_text(json.dumps({'precision':80, 'kind':'original numerical protocol vectors',
                                      'vectors':vectors}, separators=(',',':'))+'\n')
    print(f'{len(vectors)} rate/start/stop pairs; '
          f'{sum(v["bounds"] is not None for v in vectors)} legal bounds; '
          f'{sum(t is not None for v in vectors for t in v["tables"])} composed table acceptances')

if __name__ == '__main__':
    main()
