#!/usr/bin/env python3
"""Own ordinary AOT4 transitions, sparse scalar transforms and float history."""
import json, math, struct
from generate_aac_main_tools_fixtures import channel, ics
from generate_aac_ssr_fixtures import windows
from generate_he_aac_packet_fixtures import DEST, field, frequency, packed, video_fixture
GAINS=[.570829,.696616,.813004,.911304,.984900,1.067894,1.194601,1.369533]
SEQUENCES=[0,0,1,2,2,3,0,1,2,3,0,0]
def f32(x): return struct.unpack('<f',struct.pack('<f',x))[0]
class Oracle:
    def __init__(self,n):
        self.n=n; self.previous=0; self.overlap=[0.]*n; self.history=[0.]*(4*n)
        self.kbd={n:windows(n,4),n//8:windows(n//8,6)}
        self.cos={size:[[math.cos(math.pi/size*(i+.5+size/2)*(k+.5)) for i in range(2*size)] for k in range(8)] for size in (n,n//8)}
    def weight(self,size,i,shape):
        return self.kbd[size][i] if shape else math.sin(math.pi*(i+.5)/(2*size))
    def long_weight(self,i,seq,shape):
        n=self.n; small=n//8; flat=(n-small)//2
        if seq==1 and i>=n:
            j=i-n; return 1. if j<flat else self.weight(small,small+j-flat,shape) if j<flat+small else 0.
        if seq==3 and i<n:
            return 0. if i<flat else self.weight(small,i-flat,self.previous) if i<flat+small else 1.
        return self.weight(n,i,self.previous if i<n else shape)
    def run(self,seq,shape,q,active,lag,coefficient,used,skip_short_history=False):
        n=self.n; small=n//8; flat=(n-small)//2
        spectra=[[v*1024. for v in row] for row in q]
        if active:
            estimate=[v*GAINS[coefficient]*self.long_weight(i,seq,shape) for i,v in enumerate(self.history[2*n-lag:4*n-lag])]
            prediction=[sum(v*c for v,c in zip(estimate,self.cos[n][k])) for k in range(8)]
            spectra[0]=[f32(v+prediction[k]) if used[k//4] else v for k,v in enumerate(spectra[0])]
        block=[0.]*(2*n)
        if seq==2:
            for w,spectrum in enumerate(spectra):
                for i in range(2*small):
                    value=2/small*sum(v*self.cos[small][k][i] for k,v in enumerate(spectrum))
                    block[flat+w*small+i]+=value*self.weight(small,i,self.previous if w==0 and i<small else shape)
        else:
            block=[2/n*sum(v*self.cos[n][k][i] for k,v in enumerate(spectra[0]))*self.long_weight(i,seq,shape) for i in range(2*n)]
        raw=[a+b for a,b in zip(self.overlap,block[:n])]; self.overlap=block[n:]
        if not (skip_short_history and seq==2):self.history=self.history[n:2*n]+raw+self.overlap+[0.]*n
        self.previous=shape
        return [f32(v/65536) for v in raw]
def main():
    blob=bytearray();gold=bytearray();wrong=bytearray();cases=[]
    for n in (960,1024):
        oracle=Oracle(n);mutant=Oracle(n);rows=[]
        for frame,seq in enumerate(SEQUENCES):
            shape=frame%2;active=seq!=2 and frame>=1;lag=n-13*(frame%3);coefficient=frame%8;used=[True,frame%3!=1]
            q=[[(-1 if (frame+w+k)%3==0 else 1 if (frame+w+k)%3==1 else 0) for k in range(8)] for w in range(8 if seq==2 else 1)]
            info=ics(seq,2,False,shape=shape)
            if active: info=info[:-1]+'11'+field(lag,11)+field(coefficient,3)+''.join(str(int(v)) for v in used)
            packet=packed('0000000'+channel(seq,[1,1],q,info=info)+'111')
            pcm=oracle.run(seq,shape,q,active,lag,coefficient,used)
            wrong.extend(struct.pack('<'+str(n)+'f',*mutant.run(seq,shape,q,active,lag,coefficient,used,True)))
            rows.append(dict(offset=len(blob),bytes=len(packet),reference_offset=len(gold),sequence=seq,shape=shape,active=active,lag=lag,coefficient=coefficient,used=used))
            blob.extend(packet);gold.extend(struct.pack('<'+str(n)+'f',*pcm))
        case=dict(n=n,asc=packed(field(4,5)+frequency(24000)+'0001'+field(n==960,1)+'00').hex(),frames=rows,container_rate=24000,container_frame_samples=n,channels=1,slots=16,bands=32,samples=12*n,pcm_offset=0)
        case['video']=video_fixture([case],blob,filename=f'aac-ltp-transitions-{n}-synthetic.mp4');cases.append(case)
    (DEST/'aac-ltp-transitions-packets.bin').write_bytes(blob)
    (DEST/'aac-ltp-transitions-reference.f32le').write_bytes(gold)
    (DEST/'aac-ltp-transitions-stale-short-reference.f32le').write_bytes(wrong)
    (DEST/'aac-ltp-transitions.json').write_text(json.dumps(dict(cases=cases,provenance='Own AOT4 long/start/eight-short/stop packets; direct sparse cosine sums, sine/KBD and scalar float history. No private or foreign codec data.'),indent=2)+'\n')
    print('generated two LTP transition videos and 24 PCM frames')
if __name__=='__main__':main()
