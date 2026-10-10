#!/usr/bin/env python3
"""Direct Main IMDCT and SBR convolutions for the authored six-frame program.
No decoder calls; only numeric normative window/noise constants are reused.
Fixed geometry: k0=10,k2=27, off inverse filtering, fine E=128,Q=.5,
one limiter band, gain mode2, enabled smoothing, no harmonics/attack.
"""
import math,re,struct,json,hashlib
from generate_aac_main_prediction_fixtures import step,initial,f32
from generate_aac_ssr_fixtures import DEST
from generate_aac_sbr_frequency_oracles import tables
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

def patch_sources():
    # ISO Figure 4.48 over independently computed frequency borders. The odd
    # 17-band span has a final width-two master interval, not 17 unit intervals.
    master=tables(10,27,0,False,0,0)[0]
    low=10;target=10;search=len(master)-1;patches=[]
    while target<27:
        edge=max(b for b in master[:search+1] if b<=9+low-((b+8)%2))
        width=max(0,edge-target)
        if width:
            patches.append((10-((edge+8)%2)-width,target,width))
            target=edge;low=edge
        else:low=10
        if master[search]-edge<3:search=len(master)-1
    if len(patches)>1 and patches[-1][2]<3:patches.pop()
    assert patches==[(2,10,8),(2,18,7)]
    return {target+i:source+i for source,target,width in patches for i in range(width)}

def reference(prediction=True, pcm_override=None, bands=64, first_sbr_frame=0, sbr_frames=None, qmf_delay=6, frame_samples=1024):
    assert frame_samples in (960,1024)
    assert pcm_override is not None or frame_samples==1024
    qmf_rows=frame_samples//32
    assert bands in (32,64)
    assert 0 <= first_sbr_frame < 6
    enabled = [i >= first_sbr_frame for i in range(6)] if sbr_frames is None else list(sbr_frames)
    assert len(enabled) == 6
    width = 32 if not all(enabled) else 10
    source=Path(__file__).resolve().parents[1]/'crates/fvid-media/src/owned_aac/aac_sbr_qmf_window.rs'
    window=[float(x) for x in re.findall(r'-?\d+\.\d+',source.read_text().split('= [',1)[1])]
    noise_bytes=(DEST/'aac-sbr-noise-protocol.f64le').read_bytes()
    noise=[complex(*struct.unpack_from('<dd',noise_bytes,16*i)) for i in range(512)]
    pcm=core(prediction) if pcm_override is None else pcm_override;analysis=[]
    assert len(pcm)==6*frame_samples
    factors=[[window[2*lag]*complex(math.cos(math.pi*(b+.5)*(2*(lag%64)-.5)/64),math.sin(math.pi*(b+.5)*(2*(lag%64)-.5)/64))*65536 for lag in range(320)] for b in range(width)]
    for last in range(31,len(pcm),32):
        analysis.append([complex(math.fsum(pcm[last-lag]*factors[b][lag].real for lag in range(min(320,last+1))),math.fsum(pcm[last-lag]*factors[b][lag].imag for lag in range(min(320,last+1)))) for b in range(width)])
    delayed=[[0j]*width for _ in range(qmf_delay)]+analysis
    # Figure 4.48 gives 8-band and 7-band patches. The final two-band
    # patch at 25 is discarded; unpatched bands still receive envelope noise.
    mapping=patch_sources()
    weights=[.33333333333333,.30150283239582,.21816949906249,.11516383427084,.03183050093751]
    levels=[];rows=[];history=[];active_frame=0
    for frame in range(6):
        low=delayed[frame*qmf_rows:(frame+1)*qmf_rows]
        if not enabled[frame]:
            rows.extend(low)
            continue
        high=[[r[mapping[k]] if k in mapping else 0j for k in range(10,27)] for r in low]
        energy=[math.fsum(abs(r[k])**2 for r in high)/qmf_rows for k in range(17)]
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
            row=low[t][:10]+[blended[k][0]*r[k]+blended[k][1]*noise[((active_frame*qmf_rows+t)*17+k+1)%512] for k in range(17)]
            if width == 32: row += [0j]*5
            rows.append(row);history=(history+[level])[-4:]
        active_frame += 1
    synthesis_width = 32 if width == 32 else 27
    terms=[[[window[(64//bands)*(bands*lag+k)]*complex(math.cos(math.pi*(b+.5)*(2*(k+bands*(lag%2))-(255 if bands==64 else 127.5))/(2*bands)),math.sin(math.pi*(b+.5)*(2*(k+bands*(lag%2))-(255 if bands==64 else 127.5))/(2*bands)))/64 for b in range(synthesis_width)] for k in range(bands)] for lag in range(10)]
    output=[math.fsum((rows[t-lag][b]*terms[lag][k][b]).real for lag in range(min(10,t+1)) for b in range(synthesis_width))/32768 for t in range(6*qmf_rows) for k in range(bands)]
    return output

def main():
    output=reference();raw=struct.pack('<'+str(len(output))+'d',*output)
    (DEST/'aac-main-sbr-reference.f64le').write_bytes(raw)
    control=reference(False)
    control_raw=struct.pack('<'+str(len(control))+'d',*control)
    (DEST/'aac-main-sbr-no-prediction-control.f64le').write_bytes(control_raw)
    (DEST/'aac-main-sbr-reference.json').write_text(json.dumps(dict(samples=len(output),sha256=hashlib.sha256(raw).hexdigest(),control_sha256=hashlib.sha256(control_raw).hexdigest(),prediction_max_difference=max(abs(a-b) for a,b in zip(output,control)),source='ISO/IEC14496-3 4.6.18; independent Main scalar predictor/direct IMDCT, direct QMF convolution and fixed authored SBR geometry'),indent=2)+'\n')
if __name__=='__main__':main()
