#!/usr/bin/env python3
"""Own windowed direct-cosine MDCT references, independent of production FFT."""
import json,math,struct
from generate_aac_ssr_fixtures import windows
from generate_he_aac_packet_fixtures import DEST
def main():
    raw=bytearray();gold=bytearray();cases=[]
    for n in (960,1024):
        small=n//8;flat=(n-small)//2;kbd={n:windows(n,4),small:windows(small,6)}
        def weight(size,j,shape):return kbd[size][j] if shape else math.sin(math.pi*(j+.5)/(2*size))
        for seq,previous,current,dense in [(seq,a,b,False) for seq in (0,1,3) for a in (0,1) for b in (0,1)]+[(0,0,1,True),(0,1,0,True)]:
            x=[0.]* (2*n)
            if dense:x=[math.sin(i*.037)*700+math.cos(i*.019)*31 for i in range(2*n)]
            else:
                positions=sorted(set([0,1,n//2-1,n//2,n-1,n,n+1,flat-1,flat,flat+small-1,flat+small,n+flat-1,n+flat,n+flat+small-1,n+flat+small,2*n-1]))
                for j,i in enumerate(positions):x[i]=(-1 if j%2 else 1)*(73+j*127+.125)
            weighted=[]
            for i,value in enumerate(x):
                if seq==1 and i>=n:
                    j=i-n;gain=1. if j<flat else weight(small,small+j-flat,current) if j<flat+small else 0.
                elif seq==3 and i<n:gain=0. if i<flat else weight(small,i-flat,previous) if i<flat+small else 1.
                else:gain=weight(n,i,previous if i<n else current)
                if value and gain:weighted.append((i,value*gain))
            bins=[0,1,n//2,n-1] if dense else list(range(n))
            reference=[sum(value*math.cos(math.pi/n*(i+.5+n/2)*(k+.5)) for i,value in weighted) for k in bins]
            c=dict(n=n,sequence=seq,previous=previous,current=current,dense=dense,bins=bins,input_offset=len(raw),reference_offset=len(gold));cases.append(c)
            raw.extend(struct.pack('<'+str(2*n)+'d',*x));gold.extend(struct.pack('<'+str(len(reference))+'d',*reference))
    (DEST/'aac-ltp-analysis-input.f64le').write_bytes(raw)
    (DEST/'aac-ltp-analysis-reference.f64le').write_bytes(gold)
    (DEST/'aac-ltp-analysis.json').write_text(json.dumps(dict(cases=cases,provenance='Own direct MDCT sums, authored sparse boundary/dense harmonic input and independent scalar sine/KBD windows; no foreign codec execution or private media.'),indent=2)+'\n')
if __name__=='__main__':main()
