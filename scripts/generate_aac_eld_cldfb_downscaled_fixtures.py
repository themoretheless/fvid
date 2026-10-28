"""Own direct-convolution oracle for centered downscaled LD-SBR filterbanks.
Generation is separate from tests; only Python standard library is used.
"""
from functools import lru_cache
import cmath
import json
import math
import struct
from generate_aac_eld_cldfb_fixtures import ROOT, window
OUT = ROOT / 'tests/fixtures/playback-errors/aac-eld-cldfb-downscaled'

def prototype(b):
    c = window()
    if b == 64:
        return c
    stride = 64 // b
    return [(c[stride*i+(stride-1)//2]+c[stride*i+(stride-1)//2+1])/2 for i in range(10*b)]

@lru_cache(None)
def analysis_weights(b):
    c=prototype(b)
    return [[c[lag]*2*cmath.exp(1j*math.pi*(k+.5)*(2*(lag%(2*b))-(3*b-1))/(2*b)) for lag in range(10*b)]for k in range(b)]
def analysis(pcm, b):
    # Time-indexed direct convolution, no history shift or folded intermediate.
    weights=analysis_weights(b)
    return [[sum(pcm[t*b+b-1-lag]*weights[k][lag] for lag in range(min(10*b, (t+1)*b))) if k < b else 0j
             for k in range(32)] for t in range(len(pcm)//b)]

@lru_cache(None)
def synthesis_weights(b):
    return [[cmath.exp(1j*math.pi*(k+.5)*(2*n-(b-1))/(2*b))/64 for k in range(b)]for n in range(2*b)]
def synthesis(rows, b):
    c = prototype(b)
    weights=synthesis_weights(b)
    heads = [[sum((row[k]*weights[n][k]).real for k in range(b))
              for n in range(2*b)] for row in rows]
    return [sum((heads[t-2*p][k]*c[2*b*p+k] if t>=2*p else 0)+
                (heads[t-2*p-1][b+k]*c[2*b*p+b+k] if t>=2*p+1 else 0)
                for p in range(5)) for t in range(len(rows)) for k in range(b)]

def pack(values, kind='d'):
    return struct.pack('<'+kind*len(values), *values)

def generate():
    files = {}; cases = []
    for b in [8,16,32]:
        for signal in ['boundary', 'dense', 'tones']:
            name = f'a{b}-{signal}'
            pcm = [0.0]*(37*b)
            if signal == 'boundary':
                pcm[0]=1;pcm[b-1]=-.5;pcm[b]=.25;pcm[16*b-1]=.75
            else:
                pcm = [(.7*math.sin(.113*i)+.2*math.cos(.017*i)) if signal=='dense' else
                       .25*math.sin(2*math.pi*3*i/(2*b))+.125*math.cos(2*math.pi*(b-2)*i/(2*b)) for i in range(len(pcm))]
            raw = pack(pcm,'f');pcm=struct.unpack('<'+'f'*len(pcm),raw)
            rows = analysis(pcm,b)
            files[name+'.f32le'] = raw
            files[name+'.qmf.f64le'] = pack([x for row in rows for v in row for x in [v.real,v.imag]])
            cases.append(dict(name=name,kind='analysis',bands=b,rows=len(rows)))
    for b in [8,16,32,64]:
        name=f's{b}-complex'
        rows=[[complex(math.sin(.37*(t+1)*(k+1)),math.cos(.13*(t+2)*(k+3))) for k in range(64)] for t in range(37)]
        files[name+'.qmf.f64le']=pack([x for row in rows for v in row for x in [v.real,v.imag]])
        files[name+'.f64le']=pack(synthesis(rows,b))
        cases.append(dict(name=name,kind='synthesis',bands=b,rows=len(rows)))
    files['manifest.json']=(json.dumps(dict(cases=cases,provenance='Own direct time convolution and modulated-column superposition; centered prototype numeric facts. Synthetic signals only; no codec implementation, process or network.'),indent=2)+'\n').encode()
    return files

if __name__ == '__main__':
    OUT.mkdir(exist_ok=True)
    for name,data in generate().items():
        (OUT/name).write_bytes(data)
