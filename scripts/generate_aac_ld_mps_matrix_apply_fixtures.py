#!/usr/bin/env python3
"""Offline integer complex-product oracle; production code is not imported."""
import json, pathlib, struct
ROOT = pathlib.Path(__file__).resolve().parents[1]
OUT = ROOT / 'tests/fixtures/playback-errors/aac-ld-mps-matrix-apply'

def coefficient(t, b, p, r, c):
    return (t * 17 + b * 13 + p * 11 + r * 7 + c * 3) % 29 - 14

def sample(t, c, q):
    return (t * 7 + c * 5 + q * 3) % 23 - 11, (t * 11 + c * 3 + q * 7) % 19 - 9

def main():
    facts = json.loads((ROOT / 'tests/fixtures/playback-errors/aac-ld-mps-prepare/facts.json').read_text())
    cases, data = [], bytearray()
    for bands in (4, 5, 7, 9, 12, 15, 23):
        for bins in (32, 64, 128):
            mapping = facts[f'QMF{bands}'][:bins]
            for rows, cols, planes, real, imag in ((1, 1, 1, 0, None), (3, 2, 2, 0, 1), (8, 13, 7, 5, 6)):
                offset = len(data)
                for t in range(2):
                    for r in range(rows):
                        for q, b in enumerate(mapping):
                            # Exact integer complex multiplication; conversion
                            # to dyadic f64 is only done by the Rust assertion.
                            terms = [(sample(t,c,q), coefficient(t,b,real,r,c),
                                      0 if imag is None else coefficient(t,b,imag,r,c)) for c in range(cols)]
                            a = sum(x * re - y * im for (x,y),re,im in terms)
                            z = sum(y * re + x * im for (x,y),re,im in terms)
                            data.extend(struct.pack('<qq', a, z))
                cases.append(dict(bands=bands,bins=bins,rows=rows,columns=cols,planes=planes,real=real,imag=imag,offset=offset))
    OUT.mkdir(parents=True, exist_ok=True)
    (OUT / 'expected.bin').write_bytes(data)
    (OUT / 'manifest.json').write_text(json.dumps(dict(scope='Complex matrix component only; no spatial movie or player acceptance.',coefficient_denominator=32,sample_denominator=16,cases=cases),indent=2)+'\n')

if __name__ == '__main__': main()
