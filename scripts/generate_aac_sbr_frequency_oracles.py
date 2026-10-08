#!/usr/bin/env python3
"""Explicit original SBR geometry oracles from ISO equations, not a codec.

Decimal 80-digit evaluation independently checks Rust's binary-float rounding.
No network, encoder, decoder, private source or test-time generation is used.
"""
from decimal import Decimal as D, localcontext, ROUND_HALF_UP
from pathlib import Path
import json

def nint(value):
    return int(value.to_integral_value(rounding=ROUND_HALF_UP))
def geometric(a, b, count):
    if count == 0: raise ValueError()
    borders = [nint(D(a)*(D(b)/D(a))**(D(i)/D(count))) for i in range(count+1)]
    widths = sorted(y-x for x,y in zip(borders,borders[1:]))
    if min(widths) <= 0: raise ValueError()
    return widths
def tables(a, b, scale, altered, cross, density):
    if not 0 < a < b <= 64: raise ValueError()
    if scale == 0:
        step = 1+int(altered)
        count = 2*(nint(D(b-a)/D(2*step)) if altered else (b-a)//2)
        if not count: raise ValueError()
        width = [step]*count
        delta = b-a-sum(width)
        if delta > 0: width[-1] += delta
        else:
            for i in range(-delta): width[i] -= 1
    else:
        bands = D([0,12,10,8][scale])
        split = D(b)/D(a) > D('2.2449')
        mid = 2*a if split else b
        count = 2*nint(bands*(D(mid)/D(a)).ln()/(2*D(2).ln()))
        width = geometric(a,mid,count)
        if split:
            warp = D('1.3') if altered else D(1)
            count = 2*nint(bands*(D(b)/D(mid)).ln()/(2*D(2).ln()*warp))
            upper = geometric(mid,b,count)
            if min(upper) < max(width):
                shift = min(max(width)-min(upper),(max(upper)-min(upper))//2)
                upper[0] += shift
                upper[-1] -= shift
            width += sorted(upper)
    master = [a]
    for w in width: master.append(master[-1]+w)
    if cross >= len(width): raise ValueError()
    high = master[cross:]
    if high[0] > 32: raise ValueError()
    n = len(high)-1
    low = [high[0]] + [high[2*i-n%2] for i in range(1,(n+1)//2+1)]
    count = max(1,nint(D(density)*(D(b)/D(high[0])).ln()/D(2).ln()))
    if count > 5 or count >= len(low): raise ValueError()
    indices = [0]
    for i in range(count): indices.append(indices[-1]+(len(low)-1-indices[-1])//(count-i))
    return [master,high,low,[low[i] for i in indices]]

def main():
    with localcontext() as ctx:
        ctx.prec = 80
        cases = []
        for a in [8,11,17,24]:
            for b in [24,35,47,64]:
                for scale in range(4):
                    for altered in [False,True]:
                        for cross in [0,1,5]:
                            for density in [0,1,3]:
                                args = [a,b,scale,altered,cross,density]
                                try: expected = tables(*args)
                                except ValueError: expected = None
                                cases.append({'parameters':args,'expected':expected})
    path = Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/aac-sbr-frequency-oracles.json'
    path.write_text(json.dumps({'precision':80,'vectors':cases},separators=(',',':'))+'\n')
    print(f'{len(cases)} original Decimal vectors, {sum(c["expected"] is not None for c in cases)} accepted geometries')

if __name__ == '__main__':
    main()
