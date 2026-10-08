#!/usr/bin/env python3
"""Original SBR payload -> PCM oracles with silent supplied core PCM.

No AAC encoder/decoder, FFmpeg, libav, private media or network. Decimal
computes constant noise-only envelope gain/boost. Synthesis uses direct
contribution of each earlier QMF column, without shifted production histories.
Only numeric normative window/noise tables are read from saved local assets.
This is complete non-scalable SBR DSP acceptance, not full encoded HE-AAC.
"""
from decimal import Decimal as D, localcontext
from pathlib import Path
import hashlib, json, math, re, struct
from generate_aac_sbr_data_fixtures import field, word
from generate_aac_sbr_extension_fixtures import crc
from generate_aac_sbr_frequency_oracles import tables

ROOT = Path(__file__).resolve().parents[1]
DEST = ROOT / 'tests/fixtures/playback-errors'

def header(mode, smoothing):
    # fine amplitude, start=2, stop=3, crossover=0; both extras present;
    # scale=0, alter=0, noise=0, limiter density=mode/gains=2, interpolate=1.
    return '0'+field(2,4)+field(3,4)+'000'+'00'+'11'+'00'+'0'+'00'+field(mode,2)+'10'+'1'+field(smoothing,1)

def payload(nhigh, mode, smoothing, frame):
    temporal = frame > 0
    env = word(0,0)*nhigh if temporal else field(2,7)+word(1,0)*(nhigh-1)
    # Fine envelope book: time=0, frequency=1. Noise time=8 / absolute=5.
    noise = word(8,0) if temporal else field(7,5)
    data = '0'+'00001'+field(temporal,1)*2+'00'+env+noise+'0'+'0'
    body = field(frame==0,1)+(header(mode,smoothing) if frame==0 else '')+data
    protected = frame==1
    body += '0'*(-(4+10*protected+len(body))%8)
    text = field(14 if protected else 13,4)+(field(crc(body),10) if protected else '')+body
    return bytes(int(text[i:i+8],2) for i in range(0,len(text),8))

def synthesis(high, bands, window):
    # Each band contributes directly to output sample k at time t with lag d.
    # This is the normative convolution, not the implementation's v/u/g buffers.
    terms = [[[window[(64//bands)*(bands*lag+k)] *
                complex(math.cos(math.pi*(b+.5)*(2*(k+bands*(lag%2))-(255 if bands==64 else 127.5))/(2*bands)),
                        math.sin(math.pi*(b+.5)*(2*(k+bands*(lag%2))-(255 if bands==64 else 127.5))/(2*bands))) / 64
               for b in range(10,27)] for k in range(bands)] for lag in range(10)]
    result=[]
    for t in range(len(high)):
        for k in range(bands):
            result.append(math.fsum((high[t-lag][b] * terms[lag][k][b]).real
                for lag in range(min(10,t+1)) for b in range(17))/32768)
    return result

def main():
    source=(ROOT/'crates/fvid-media/src/owned_aac/aac_sbr_qmf_window.rs').read_text()
    window=[float(x) for x in re.findall(r'-?\d+\.\d+',source.split('= [',1)[1])]
    assert len(window)==640
    raw_noise=(DEST/'aac-sbr-noise-protocol.f64le').read_bytes()
    noise=[complex(*struct.unpack_from('<dd',raw_noise,i*16)) for i in range(512)]
    syntax=bytearray(); pcm=bytearray(); cases=[]
    with localcontext() as ctx:
        ctx.prec=80
        _, high, _, _ = tables(10,27,0,False,0,0)
        nhigh=len(high)-1
        # All bands have EOrig=128, QOrig=.5, ECurr=0, no harmonic/attack.
        # Every limiter interval has the same ratio, independent of its width.
        amplitude=(D(128)*D('.5')/D('1.5')).sqrt()*min(D(3).sqrt(),D('1.584893192'))
        for slots in [15,16]:
            for mode in range(4):
                for smoothing in [False,True]:
                    frames=[]
                    for frame in range(3):
                        data=payload(nhigh,mode,smoothing,frame)
                        frames.append(dict(offset=len(syntax),byte_length=len(data)))
                        syntax.extend(data)
                    gain=amplitude if smoothing else amplitude*D('0.99999999999999')
                    # RATE*tE(last) == nominal frame length; no overhang. The
                    # noise index is continuous across all three frames.
                    hf=[[float(gain)*noise[(t*17+b+1)%512] for b in range(17)] for t in range(6*slots)]
                    for bands in [32,64]:
                        result=synthesis(hf,bands,window)
                        output=struct.pack('<'+str(len(result))+'d',*result)
                        cases.append(dict(slots=slots,limiter=mode,smoothing=smoothing,bands=bands,
                            frames=frames,pcm_offset=len(pcm),samples=len(result),sha256=hashlib.sha256(output).hexdigest()))
                        pcm.extend(output)
    (DEST/'aac-sbr-dsp-syntax.bin').write_bytes(syntax)
    (DEST/'aac-sbr-dsp-pcm.f64le').write_bytes(pcm)
    (DEST/'aac-sbr-dsp-oracles.json').write_text(json.dumps(dict(
        kind='original three-frame SBR payload to PCM; silent supplied core, not full HE-AAC',
        precision=80,syntax_sha256=hashlib.sha256(syntax).hexdigest(),pcm_sha256=hashlib.sha256(pcm).hexdigest(),
        noise_sha256=hashlib.sha256(raw_noise).hexdigest(),cases=cases),separators=(',',':'))+'\n')
    print(len(cases),'three-frame payload-to-PCM traces;',len(syntax),'syntax bytes;',len(pcm),'PCM bytes')

if __name__=='__main__': main()
