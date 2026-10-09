#!/usr/bin/env python3
"""Own TNS FIR reference by direct convolution of immutable spectral regions."""
import json,struct
from generate_he_aac_packet_fixtures import DEST
def main():
    raw=bytearray();gold=bytearray();cases=[]
    for n in (960,1024):
        for count in (1,8):
            size=n//count;offsets=[0,4,8,16,32,size]
            for order in ((0,1,7,12,20) if count==1 else (0,1,7)):
                for reverse in (False,True):
                    for limit in (0,2,5):
                        source=[((i*13)%47-23)*.125 for i in range(n)];expected=source[:];windows=[]
                        for w in range(count):
                            filters=[dict(length=2,reverse=reverse^(w%2==1),lpc=[(-1)**j*.125/(j+1) for j in range(order)]),dict(length=3,reverse=not reverse,lpc=[.25,-.0625])];windows.append(filters)
                            top=len(offsets)-1
                            for f in filters:
                                bottom=max(0,top-f['length']);indices=list(range(w*size+offsets[min(bottom,limit)],w*size+offsets[min(top,limit)]));top=bottom
                                if f['reverse']:indices.reverse()
                                original=[expected[i] for i in indices]
                                for t,i in enumerate(indices):expected[i]=original[t]+sum(a*original[t-1-j] for j,a in enumerate(f['lpc']) if j<t)
                        cases.append(dict(n=n,offsets=offsets,limit=limit,windows=windows,input_offset=len(raw),reference_offset=len(gold)))
                        raw.extend(struct.pack('<'+str(n)+'d',*source));gold.extend(struct.pack('<'+str(n)+'d',*expected))
    (DEST/'aac-tns-analysis-input.f64le').write_bytes(raw)
    (DEST/'aac-tns-analysis-reference.f64le').write_bytes(gold)
    (DEST/'aac-tns-analysis.json').write_text(json.dumps(dict(cases=cases,provenance='Own direct immutable-region FIR convolution, clipping and per-filter/window resets; no foreign codec execution or private media.'),indent=2)+'\n')
if __name__=='__main__':main()
