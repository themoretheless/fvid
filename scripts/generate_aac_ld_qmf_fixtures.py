"""Own scalar LD-QMF traces. Explicit offline generation; no processes/network.
The window.json numeric facts have separate official-ISO provenance.
"""
import cmath,json,math,struct
from pathlib import Path
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/aac-ld-qmf'
def pack(values,kind):return struct.pack('<'+kind*len(values),*values)
def f32(v):return struct.unpack('<f',struct.pack('<f',v))[0]
def generate():
    window=json.loads((OUT/'window.json').read_text());files={};cases=[];synthesis_cases=[]
    for n in [32,64]:
        c=window if n==64 else [(window[2*i]+window[2*i+1])/2 for i in range(320)]
        a=[[64/n*cmath.exp(1j*math.pi*(k+.5)*(2*j+2*n+1)/(2*n)) for j in range(2*n)]for k in range(n)]
        s=[[cmath.exp(1j*math.pi*(k+.5)*(2*j-4*n+1)/(2*n))/64 for k in range(n)]for j in range(2*n)]
        for name in ['impulse','tones','dense']:
            pcm=[f32((1. if i==0 else 0.)if name=='impulse' else (.13*math.sin(.11*i)+.07*math.cos(.79*i))if name=='tones' else (((i*17+3)%101)-50)/100) if i<3*n else 0. for i in range(40*n)]
            slots=[]
            for t in range(40):
                anchor=(t+1)*n-1
                slots.append([sum(pcm[anchor-i]*c[i]*a[k][i%(2*n)] for i in range(min(10*n,anchor+1))) for k in range(n)])
            # Absolute slot timeline, no production overlap/history shifting.
            transformed=[[sum(v[k]*s[j][k] for k in range(n)).real for j in range(2*n)]for v in slots]
            pcm_out=[]
            for t in range(40):
                for k in range(n):
                    pcm_out.append(sum((transformed[t-2*p][k]*c[2*n*p+k] if t>=2*p else 0.)+(transformed[t-2*p-1][n+k]*c[2*n*p+n+k] if t>=2*p+1 else 0.)for p in range(5)))
            key=f'{n}-{name}';files[key+'.input.f32le']=pack(pcm,'f');files[key+'.analysis.f64le']=pack([x for row in slots for v in row for x in [v.real,v.imag]],'d');files[key+'.output.f64le']=pack(pcm_out,'d');cases.append(dict(name=key,bands=n,slots=40,peak=max(range(len(pcm_out)),key=lambda i:abs(pcm_out[i]))))
        for name in ['low-real','high-imag','complex-dense']:
            rows=[]
            for t in range(40):
                rows.append([complex(f32((((t*11+k*7)%23)-11)/30),f32((((t*17+k*3)%31)-15)/40))if name=='complex-dense' else complex(1. if k==0 and t==0 else 0.,0.)if name=='low-real' else complex(0.,1. if k==n-1 and t==3 else 0.)for k in range(n)] if t<5 else [0j]*n)
            v=[[sum(row[k]*s[j][k]for k in range(n)).real for j in range(2*n)]for row in rows]
            output=[sum((v[t-2*p][k]*c[2*n*p+k]if t>=2*p else 0.)+(v[t-2*p-1][n+k]*c[2*n*p+n+k]if t>=2*p+1 else 0.)for p in range(5))for t in range(40)for k in range(n)]
            key=f'{n}-{name}';files[key+'.complex.f32le']=pack([x for row in rows for z in row for x in [z.real,z.imag]],'f');files[key+'.output.f64le']=pack(output,'d');synthesis_cases.append(dict(name=key,bands=n,slots=40))
    files['manifest.json']=(json.dumps(dict(cases=cases,synthesis_cases=synthesis_cases,provenance='Own absolute-time scalar convolution, complex exponential modulation, impulses, tones and deterministic dense bursts. LD-QMF foundation only; not spatial playback acceptance.'),indent=2)+'\n').encode();return files
if __name__=='__main__':
    for name,data in generate().items():(OUT/name).write_bytes(data)
