#!/usr/bin/env python3
"""Direct Main IMDCT and SBR convolutions for the authored six-frame program.
No decoder calls; only numeric normative window/noise constants are reused.
Fixed geometry: k0=10,k2=27, off inverse filtering, fine E=128,Q=.5,
one limiter band, gain mode2, enabled smoothing, no harmonics/attack.
"""
import math,re,struct,json,hashlib
from generate_aac_main_prediction_fixtures import step,initial,f32
from generate_aac_ssr_fixtures import DEST
from pathlib import Path

def core(prediction=True):
    bank=[initial() for _ in range(4)];overlap=[0.]*1024;pcm=[]
    for frame in range(6):
        residual=[1,-1,1,-1] if frame%2 else [-1,1,-1,1]
        coefficients=[]
        for k,x in enumerate(residual):
            value,bank[k]=step(bank[k],float(x*1024),prediction and frame>=3);coefficients.append(value)
        block=[2/1024*math.fsum(x*math.cos(math.pi/1024*(j+.5+512)*(k+.5)) for k,x in enumerate(coefficients))*math.sin(math.pi*(j+.5)/2048) for j in range(2048)]
        pcm.extend(f32(f32(overlap[j]+block[j])/65536) for j in range(1024));overlap=block[1024:]
    return pcm

def reference(prediction=True):
    source=Path(__file__).resolve().parents[1]/'crates/fvid-media/src/owned_aac/aac_sbr_qmf_window.rs'
    window=[float(x) for x in re.findall(r'-?\d+\.\d+',source.read_text().split('= [',1)[1])]
    noise_bytes=(DEST/'aac-sbr-noise-protocol.f64le').read_bytes()
    noise=[complex(*struct.unpack_from('<dd',noise_bytes,16*i)) for i in range(512)]
    pcm=core(prediction);analysis=[]
    factors=[[window[2*lag]*complex(math.cos(math.pi*(b+.5)*(2*(lag%64)-.5)/64),math.sin(math.pi*(b+.5)*(2*(lag%64)-.5)/64))*65536 for lag in range(320)] for b in range(10)]
    for last in range(31,len(pcm),32):
        analysis.append([complex(math.fsum(pcm[last-lag]*factors[b][lag].real for lag in range(min(320,last+1))),math.fsum(pcm[last-lag]*factors[b][lag].imag for lag in range(min(320,last+1)))) for b in range(10)])
    delayed=[[0j]*10 for _ in range(6)]+analysis
    # Figure 4.48 gives two 8-band patches (2->10 and 2->18).
    # The final one-band patch at 26 is discarded; band26 still gets noise.
    weights=[.33333333333333,.30150283239582,.21816949906249,.11516383427084,.03183050093751]
    levels=[];rows=[];history=[]
    for frame in range(6):
        low=delayed[frame*32:frame*32+32]
        high=[[r[2+(k-10)%8] if k<26 else 0j for k in range(10,27)] for r in low]
        energy=[math.fsum(abs(r[k])**2 for r in high)/32 for k in range(17)]
        gains=[math.sqrt(128/(1.5*(1+e))) for e in energy];q=[math.sqrt(128/3)]*17
        maximum=min(math.sqrt((128*17+1e-12)/(sum(energy)+1e-12))*1.41254,1e5)
        for k,g in enumerate(gains):
            if g>maximum:q[k]*=maximum/g;gains[k]=maximum
        reconstructed=1e-12+math.fsum(e*g*g+n*n for e,g,n in zip(energy,gains,q))
        boost=min(math.sqrt((128*17+1e-12)/reconstructed),1.584893192)
        level=[(g*boost,n*boost) for g,n in zip(gains,q)];levels.append(level)
        if not history:history=[level]*4
        for t,r in enumerate(high):
            sequence=history+[level];blended=[tuple(math.fsum(weights[j]*sequence[-1-j][k][p] for j in range(5)) for p in range(2)) for k in range(17)]
            row=low[t]+[blended[k][0]*r[k]+blended[k][1]*noise[((frame*32+t)*17+k+1)%512] for k in range(17)]
            rows.append(row);history=(history+[level])[-4:]
    terms=[[[window[64*lag+k]*complex(math.cos(math.pi*(b+.5)*(2*(k+64*(lag%2))-255)/128),math.sin(math.pi*(b+.5)*(2*(k+64*(lag%2))-255)/128))/64 for b in range(27)] for k in range(64)] for lag in range(10)]
    output=[math.fsum((rows[t-lag][b]*terms[lag][k][b]).real for lag in range(min(10,t+1)) for b in range(27))/32768 for t in range(192) for k in range(64)]
    return output

def main():
    output=reference();raw=struct.pack('<'+str(len(output))+'d',*output)
    (DEST/'aac-main-sbr-reference.f64le').write_bytes(raw)
    control=reference(False)
    control_raw=struct.pack('<'+str(len(control))+'d',*control)
    (DEST/'aac-main-sbr-no-prediction-control.f64le').write_bytes(control_raw)
    (DEST/'aac-main-sbr-reference.json').write_text(json.dumps(dict(samples=len(output),sha256=hashlib.sha256(raw).hexdigest(),control_sha256=hashlib.sha256(control_raw).hexdigest(),prediction_max_difference=max(abs(a-b) for a,b in zip(output,control)),source='ISO/IEC14496-3 4.6.18; independent Main scalar predictor/direct IMDCT, direct QMF convolution and fixed authored SBR geometry'),indent=2)+'\n')
if __name__=='__main__':main()
