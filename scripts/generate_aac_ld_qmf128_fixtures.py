#!/usr/bin/env python3
"""Own absolute-time128-bin LD-QMF oracle; explicit offline generation."""
import cmath,json,math,struct
from pathlib import Path
ROOT=Path(__file__).resolve().parents[1];OUT=ROOT/'tests/fixtures/playback-errors/aac-ld-qmf128'
def pack(values,kind):return struct.pack('<'+kind*len(values),*values)
def f32(v):return struct.unpack('<f',struct.pack('<f',v))[0]
def coefficients():
    w=json.loads((OUT.parent/'aac-ld-qmf/window.json').read_text())
    # The reference declares1280 values but has640 explicit LD coefficients.
    # Its non64 branch averages adjacent cells at floor(i*64/resolution).
    return [(w[i//2]+(w[i//2+1]if i//2+1<len(w)else 0.))/2 for i in range(1280)]
def main():
    OUT.mkdir(exist_ok=True);n=128;c=coefficients();cases=[];synth=[]
    a=[[.5*cmath.exp(1j*math.pi*(k+.5)*(2*j+2*n+1)/(2*n))for j in range(2*n)]for k in range(n)]
    s=[[cmath.exp(1j*math.pi*(k+.5)*(2*j-4*n+1)/(2*n))/64 for k in range(n)]for j in range(2*n)]
    def synthesis(rows):
        v=[[sum(row[k]*s[j][k]for k in range(n)).real for j in range(2*n)]for row in rows]
        return [sum((v[t-2*p][k]*c[2*n*p+k]if t>=2*p else 0.)+(v[t-2*p-1][n+k]*c[2*n*p+n+k]if t>=2*p+1 else 0.)for p in range(5))for t in range(len(rows))for k in range(n)]
    for name in ('impulse','tones','dense'):
        pcm=[f32((1. if i==0 else 0.)if name=='impulse'else(.13*math.sin(.11*i)+.07*math.cos(.79*i))if name=='tones'else(((i*17+3)%101)-50)/100)if i<3*n else 0. for i in range(40*n)]
        rows=[]
        for t in range(40):
            anchor=(t+1)*n-1
            folded=[sum((pcm[anchor-(j+2*n*p)]if anchor>=j+2*n*p else 0.)*c[j+2*n*p]for p in range(5))for j in range(2*n)]
            rows.append([sum(v*z for v,z in zip(folded,row))for row in a])
        (OUT/(name+'.input.f32le')).write_bytes(pack(pcm,'f'));(OUT/(name+'.analysis.f64le')).write_bytes(pack([v for row in rows for z in row for v in(z.real,z.imag)],'d'));(OUT/(name+'.output.f64le')).write_bytes(pack(synthesis(rows),'d'));cases.append(dict(name=name,bands=n,slots=40))
    for name in ('low-real','high-imag','complex-dense'):
        rows=[]
        for t in range(40):rows.append([complex(f32((((t*11+k*7)%23)-11)/30),f32((((t*17+k*3)%31)-15)/40))if name=='complex-dense'else complex(1. if k==0 and t==0 else 0.,0.)if name=='low-real'else complex(0.,1. if k==n-1 and t==3 else 0.)for k in range(n)]if t<5 else[0j]*n)
        (OUT/(name+'.complex.f32le')).write_bytes(pack([v for row in rows for z in row for v in(z.real,z.imag)],'f'));(OUT/(name+'.output.f64le')).write_bytes(pack(synthesis(rows),'d'));synth.append(dict(name=name,bands=n,slots=40))
    (OUT/'manifest.json').write_text(json.dumps(dict(cases=cases,synthesis_cases=synth,scope='Own128-bin scalar bank traces; component qualification only,not whole codec acceptance.'),indent=2)+'\n');print('128-bin scalar analysis/synthesis traces generated')
if __name__=='__main__':main()
