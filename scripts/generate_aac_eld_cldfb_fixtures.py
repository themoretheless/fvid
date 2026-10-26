"""Own direct convolution/modulation oracle for normative low-delay filterbanks."""
import json,math,re,struct
from pathlib import Path
ROOT=Path(__file__).resolve().parents[1]
OUT=ROOT/'tests/fixtures/playback-errors/aac-eld-cldfb'

def window():
    text=(ROOT/'crates/fvid-media/src/owned_aac/aac_eld_cldfb_window.rs').read_text().split('= [',1)[1].split('];',1)[0]
    values=[float(v)for v in re.findall(r'-?\d+\.\d+e[+-]\d+',text)]
    assert len(values)==640
    return values

def synthesis(rows,bands,c):
    # Literal modulated-column superposition, without a shifting state/ring.
    w=c if bands==64 else[(a+b)/2 for a,b in zip(c[::2],c[1::2])]
    matrix=[[complex(math.cos(math.pi*(k+.5)*(2*n-(bands-1))/(2*bands)),math.sin(math.pi*(k+.5)*(2*n-(bands-1))/(2*bands)))/bands for k in range(bands)]for n in range(2*bands)]
    heads=[[sum((row[k]*matrix[n][k]).real for k in range(bands))for n in range(2*bands)]for row in rows]
    return [sum((heads[t-2*p][k]*w[2*bands*p+k]if t>=2*p else 0)+(heads[t-2*p-1][bands+k]*w[2*bands*p+bands+k]if t>=2*p+1 else 0)for p in range(5))for t in range(len(rows))for k in range(bands)]

def generate():
    c=window();ci=[(a+b)/2 for a,b in zip(c[::2],c[1::2])]
    files={};cases=[]
    for name in ['impulse-first','impulse-boundary','dense','tones']:
        pcm=[0.]*1280
        if name=='impulse-first':pcm[0]=1.
        elif name=='impulse-boundary':pcm[31]=.75;pcm[32]=-.25
        elif name=='dense':pcm=[math.sin(.113*i)*.7+math.cos(.017*i)*.2 for i in range(len(pcm))]
        else:pcm=[math.sin(2*math.pi*3*i/64)*.25+math.cos(2*math.pi*23*i/64)*.125 for i in range(len(pcm))]
        pcm=list(struct.unpack('<'+'f'*len(pcm),struct.pack('<'+'f'*len(pcm),*pcm)))
        rows=[]
        for t in range(len(pcm)//32):
            row=[]
            for k in range(32):
                value=0j
                for lag in range(320):
                    at=t*32+31-lag
                    if at<0:continue
                    phase=math.pi*(k+.5)*(2*(lag%64)-96)/64
                    value+=pcm[at]*ci[lag]*2*complex(math.cos(phase),math.sin(phase))
                row.append(value)
            rows.append(row+[0j]*32)
        files[name+'.f32le']=struct.pack('<'+'f'*len(pcm),*pcm)
        files[name+'.qmf.f64le']=b''.join(struct.pack('<dd',v.real,v.imag)for row in rows for v in row[:32])
        for bands in [32,64]:
            result=synthesis(rows,bands,c)
            files[name+f'.pcm{bands}.f64le']=struct.pack('<'+'d'*len(result),*result)
        cases.append(dict(name=name,kind='analysis',rows=len(rows)))
    for name in ['complex-dense','high-imaginary']:
        rows=[[complex(math.sin(.37*(t+1)*(k+1)),math.cos(.13*(t+2)*(k+3)))if name=='complex-dense'else complex(0,.125*(t%3-1))if k>=25 else 0j for k in range(64)]for t in range(24)]
        files[name+'.qmf.f64le']=b''.join(struct.pack('<dd',v.real,v.imag)for row in rows for v in row)
        for bands in [32,64]:
            result=synthesis(rows,bands,c)
            files[name+f'.pcm{bands}.f64le']=struct.pack('<'+'d'*len(result),*result)
        cases.append(dict(name=name,kind='synthesis',rows=len(rows)))
    files['manifest.json']=(json.dumps(dict(cases=cases,provenance='Own direct 320-tap time convolution, complex modulation and modulated-column superposition; normative GOST table A.90 numeric facts only. No private media, foreign codec, process or network.'),indent=2)+'\n').encode()
    return files
if __name__=='__main__':
    OUT.mkdir(exist_ok=True)
    for name,data in generate().items():(OUT/name).write_bytes(data)
