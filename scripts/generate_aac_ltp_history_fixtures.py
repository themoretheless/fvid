#!/usr/bin/env python3
"""Independent authored LTP history oracle using a time-axis piecewise signal."""
import json,struct
from generate_he_aac_packet_fixtures import DEST
GAINS=[.570829,.696616,.813004,.911304,.984900,1.067894,1.194601,1.369533]
def main():
    source=bytearray();oracle=bytearray();cases=[]
    for n in (960,1024):
        previous=[0]*n;steps=[]
        for frame in range(3):
            pcm=[((i*17+frame*13)%101-50)*801+(.5 if i%2 else -.5) for i in range(n)]
            overlap=[((i*29+frame*11)%97-48)*731+(.5 if i%3 else -.5) for i in range(n)]
            row=dict(input_offset=len(source));source.extend(struct.pack('<'+str(2*n)+'d',*(pcm+overlap)))
            current=[min(32767,max(-32768,round(x))) for x in pcm];future=[min(32767,max(-32768,round(x))) for x in overlap]
            estimates=[]
            for lag in sorted(set((0,1,n,min(2*n,2047),2*n))):
                for coefficient,gain in enumerate(GAINS):
                    values=[]
                    for i in range(2*n):
                        t=i-lag
                        sample=previous[t+2*n] if t<-n else current[t+n] if t<0 else future[t] if t<n else 0
                        values.append(sample*gain)
                    estimates.append(dict(lag=lag,coefficient=coefficient,offset=len(oracle)))
                    oracle.extend(struct.pack('<'+str(2*n)+'d',*values))
            previous=current;row['estimates']=estimates;steps.append(row)
        cases.append(dict(n=n,steps=steps))
    (DEST/'aac-ltp-history-input.f64le').write_bytes(source)
    (DEST/'aac-ltp-history-reference.f64le').write_bytes(oracle)
    (DEST/'aac-ltp-history.json').write_text(json.dumps(dict(cases=cases,provenance='Own integer-quantized piecewise time-axis history and eight gain coefficients; no foreign codec execution, network or private media.'),indent=2)+'\n')
if __name__=='__main__':main()
