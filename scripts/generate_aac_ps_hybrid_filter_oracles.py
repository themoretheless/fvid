#!/usr/bin/env python3
"""Explicit offline PS FIR oracle from normative decimals and Decimal convolution.
No media decoder, FFmpeg, network, fixture generation during tests, or Rust output.
"""
import json
from decimal import Decimal as D
from pathlib import Path
from generate_aac_ps_mixing_oracles import PI, sincos
ROOT = Path(__file__).resolve().parents[1]
DEST = ROOT / 'tests/fixtures/playback-errors'
PROTOTYPES = {
    'TwentyEight': (8, ['.00746082949812','.02270420949825','.04546865930473','.07266113929591','.09885108575264','.11793710567217','.125']),
    'TwentyTwo': (2, ['0','.01899487526049','0','-.07293139167538','0','.30596630545168','.5']),
    'ThirtyFourTwelve': (12, ['.04081179924692','.03812810994926','.05144908135699','.06399831151592','.07428313801106','.08100347892914','.08333333333333']),
    'ThirtyFourEight': (8, ['.01565675600122','.03752716391991','.05417891378782','.08417044116767','.10307344158036','.12222452249753','.125']),
    'ThirtyFourFour': (4, ['-.05908211155639','-.04871498374946','0','.07778723915851','.16486303567403','.23279856662996','.25']),
}
def main():
    rows=[]
    signals = {
        'real_impulse': [(D(int(n==0)),D(0)) for n in range(32)],
        'imaginary_impulse': [(D(0),D(int(n==11))) for n in range(32)],
        'rational_complex': [(D((13*n+7)%23-11)/16,D((3*n+11)%31-15)/32) for n in range(47)],
    }
    for name,(width,half) in PROTOTYPES.items():
        prototype=list(map(D,half));prototype+=prototype[-2::-1]
        taps=[]
        for q in range(width):
            channel=[]
            for n,g in enumerate(prototype):
                phase=2*PI*(D(q)+(D(0) if name=='TwentyTwo' else D('.5')))*(n-6)/width
                sn,cs=sincos(phase)
                channel.append((g*cs,D(0) if name=='TwentyTwo' else g*sn))
            taps.append(channel)
        for signal,input in signals.items():
            output=[]
            for n in range(len(input)):
                slot=[]
                # Explicit full linear convolution, no circular buffers or bank state.
                for channel in taps:
                    re=im=D(0)
                    for m,(xr,xi) in enumerate(input):
                        lag=n-m
                        if 0<=lag<13:
                            gr,gi=channel[lag];re+=xr*gr-xi*gi;im+=xr*gi+xi*gr
                    slot.append([float(re),float(im)])
                output.append(slot)
            rows.append(dict(prototype=name,signal=signal,input=[[float(r),float(i)] for r,i in input],output=output))
    (DEST/'aac-ps-hybrid-filter-oracles.json').write_text(json.dumps(dict(source='GOST R 53556.8-2013 6.4.3, tables 36-38; own Decimal 90 linear convolution',delay=6,rows=rows),indent=2)+'\n')
if __name__=='__main__':main()
