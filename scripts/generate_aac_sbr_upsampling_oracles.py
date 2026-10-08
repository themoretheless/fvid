#!/usr/bin/env python3
"""Original delay-only SBR PCM oracle, direct convolution, offline only."""
from pathlib import Path
import math,re,struct,json,hashlib
root=Path(__file__).resolve().parents[1];dest=root/'tests/fixtures/playback-errors'
window=[float(x) for x in re.findall(r'-?\d+\.\d+', (root/'crates/fvid-media/src/owned_aac/aac_sbr_qmf_window.rs').read_text().split('= [',1)[1])]
manifest=[]
for slots in [15,16]:
    n=64*slots*3
    pcm=[(((i*73+19)%257-128)/256 if i<n//2 else 0.0) for i in range(n)]
    raw=struct.pack('<'+str(n)+'f',*pcm)
    analysis=[]
    for last in range(31,n,32):
        analysis.append([complex(math.fsum(2*pcm[last-lag]*window[2*lag]*math.cos(math.pi*(b+.5)*(2*(lag%64)-.5)/64) for lag in range(min(320,last+1))),math.fsum(2*pcm[last-lag]*window[2*lag]*math.sin(math.pi*(b+.5)*(2*(lag%64)-.5)/64) for lag in range(min(320,last+1)))) for b in range(32)])
    # Eight retained analysis columns, select l+2: net six-column delay.
    low=[[0j]*32 for _ in range(6)]+analysis
    for bands in [32,64]:
        terms=[[[window[(64//bands)*(bands*lag+k)]*complex(math.cos(math.pi*(b+.5)*(2*(k+bands*(lag%2))-(255 if bands==64 else 127.5))/(2*bands)),math.sin(math.pi*(b+.5)*(2*(k+bands*(lag%2))-(255 if bands==64 else 127.5))/(2*bands)))/64 for b in range(32)] for k in range(bands)] for lag in range(10)]
        output=[math.fsum((low[t-lag][b]*terms[lag][k][b]).real for lag in range(min(10,t+1)) for b in range(32)) for t in range(len(analysis)) for k in range(bands)]
        expected=struct.pack('<'+str(len(output))+'d',*output)
        prefix=f'aac-sbr-upsampling-{slots}-{bands}'
        (dest/(prefix+'.f32le')).write_bytes(raw);(dest/(prefix+'.f64le')).write_bytes(expected)
        manifest.append(dict(prefix=prefix,slots=slots,bands=bands,input_sha256=hashlib.sha256(raw).hexdigest(),output_sha256=hashlib.sha256(expected).hexdigest()))
(dest/'aac-sbr-upsampling-oracles.json').write_text(json.dumps(manifest,indent=2)+'\n')
print('4 original nonzero three-frame delay-only PCM oracles')
