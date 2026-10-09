#!/usr/bin/env python3
"""Own AOT 3 videos and scalar SSR PCM oracle; offline, no foreign encoder."""
import hashlib, json, math, re, struct
from pathlib import Path
from generate_he_aac_packet_fixtures import DEST, field, frequency, packed, video_fixture
ROOT=DEST.parents[2]
def ints(name):
    source=(ROOT/'crates/fvid-media/src/owned_aac/aac_huffman_tables.rs').read_text()
    body=re.search(r'const '+name+r':[^=]+ = \[(.*?)\];',source,re.S)[1]
    return [int(n,0) for n in re.findall(r'0x[0-9a-fA-F]+|\d+',body)]
CODES,LENS=ints('SPECTRUM_CODEBOOK1_CODES'),ints('SPECTRUM_CODEBOOK1_LENS')
SC,SL=ints('SCF_CODEBOOK_CODES'),ints('SCF_CODEBOOK_LENS')
source=(ROOT/'crates/fvid-media/src/owned_aac/aac_band_tables.rs').read_text()
def offsets(kind):
    body=re.search(r'const SWB_OFFSET_24K_'+kind+r':[^=]+ = \[(.*?)\];',source,re.S)[1]
    return [int(n) for n in re.findall(r'\d+',body)]
LONG,SHORT=offsets('LONG'),offsets('SHORT')
# Factual prototype coefficients are shared, but the oracle evaluates literal
# upsampling/convolution rather than the production polyphase ring.
source=(ROOT/'crates/fvid-media/src/owned_aac/aac_ssr_ipqf.rs').read_text()
Q=[float(x) for x in re.findall(r'[-+]?\d+\.\d+e[-+]\d+',source.split('const PROTOTYPE')[1].split('];')[0])]
assert len(Q)==48
Q+=Q[::-1]
SEQUENCES=[0,1,2,2,3,0]
LENGTHS={0:[256],1:[112,32],2:[32]*8,3:[112,256]}
SAMPLES={0:1024,1:1472,2:1024,3:576}
def windows(n,a):
    def i0(x):
        total=term=1.
        for k in range(1,100):
            term*=x*x/(4*k*k);total+=term
            if term<total*2.22e-16:break
        return total
    weights=[i0(math.pi*a*math.sqrt(max(0,1-(2*i/n-1)**2))) for i in range(n+1)]
    total=sum(weights);prefix=0.;out=[]
    for w in weights[:-1]:prefix+=w;out.append(math.sqrt(prefix/total))
    return out+out[::-1]
KBD={32:windows(32,6),256:windows(256,4)}
def weight(shape,n,i):return KBD[n][i] if shape else math.sin(math.pi*(i+.5)/(2*n))
def info(seq,shape):
    swb=SHORT if seq==2 else LONG
    return '0'+field(seq,2)+field(shape,1)+field(len(swb)-1,4 if seq==2 else 6)+('1111111' if seq==2 else '0')
def adjustments(seq,active,channel):
    return [[[(5+channel+b,1)] if active else [] for _ in LENGTHS[seq]] for b in range(3)]
def gain_bits(seq,active,channel,bad=False):
    result=field(3,2)
    for band in adjustments(seq,active,channel):
        for window,points in enumerate(band):
            width=4 if window==0 and seq in (1,3) else (2 if seq in (1,2) else 5)
            result+=field(len(points),3)
            for level,location in points:result+=field(level,4)+field(14 if bad and seq==1 and window==0 else location,width)
    return result

def spectrum(frame,seq,channel):
    n=128 if seq==2 else 1024;out=[0.]*1024
    for window in range(8 if seq==2 else 1):
        for band in range(4):
            for k in range(4):out[window*n+band*(n//4)+k]=1024.*(-1 if (frame+channel+window+band)%2 else 1)
    return out

def channel(frame,seq,shape,c,active,common,bad=False):
    swb=SHORT if seq==2 else LONG
    result=field(140,8)+('' if common else info(seq,shape))+'0001'
    remain=len(swb)-1;escape=7 if seq==2 else 31;size=3 if seq==2 else 5
    while remain>=escape:result+=field(escape,size);remain-=escape
    result+=field(remain,size)
    result+=''.join(field(SC[60],SL[60]) for _ in range(len(swb)-1))
    result+='00'+'1'+gain_bits(seq,active,c,bad)
    sp=spectrum(frame,seq,c);n=128 if seq==2 else 1024
    for start,end in zip(swb,swb[1:]):
        for window in range(8 if seq==2 else 1):
            for position in range(start,end,4):
                index=0
                for k in range(4):index=index*3+int(sp[window*n+position+k]/1024)+1
                result+=field(CODES[index],LENS[index])
    return result

def packet(frame,seq,shape,channels,active,bad=False):
    bits='0000000'+channel(frame,seq,shape,0,active,False,bad) if channels==1 else '0010000'+'1'+info(seq,shape)+'00'+''.join(channel(frame,seq,shape,c,active,True,bad) for c in range(2))
    return packed(bits+'111')

def curve(points,size):
    levels=[(0,points[0][0]-4 if points else 0)]+[(8*location,level-4) for level,location in points]+[(size,0)]
    out=[]
    for j in range(size):
        m=max(k for k,(location,_) in enumerate(levels) if location<=j)
        at,a=levels[m];_,b=levels[m+1]
        t=min(j-at,8)/8
        out.append(2**((1-t)*a+t*b))
    return 2**levels[0][1],out

def oracle(shapes,channels,active,spectral_source=None):
    previous_shape=None;fragment=[[[1.]*256 for _ in range(3)] for _ in range(channels)]
    overlap=[[[0.]*256 for _ in range(4)] for _ in range(channels)]
    band_history=[[] for _ in range(channels)];out=bytearray();frames=[]
    for frame,(seq,shape) in enumerate(zip(SEQUENCES,shapes)):
        old=shape if previous_shape is None else previous_shape;rows=[]
        for c in range(channels):
            sp=(spectral_source or spectrum)(frame,seq,c);raw=[]
            for band in range(4):
                block=[];n=32 if seq==2 else 256
                for w in range(8 if seq==2 else 1):
                    base=128*w+32*band if seq==2 else 256*band
                    coeff=sp[base:base+n]
                    if band%2:coeff=coeff[::-1]
                    for j in range(2*n):
                        v=sum(x*math.cos(math.pi/n*(j+.5+n/2)*(k+.5)) for k,x in enumerate(coeff))*2/n
                        if seq==2:wt=weight(old if w==0 and j<32 else shape,32,j)
                        elif seq==1:wt=weight(old,256,j) if j<256 else (1. if j<368 else weight(shape,32,j-336) if j<400 else 0.)
                        elif seq==3:wt=0. if j<112 else weight(old,32,j-112) if j<144 else 1. if j<256 else weight(shape,256,j)
                        else:wt=weight(old if j<256 else shape,256,j)
                        block.append(v*wt)
                if band:
                    curves=[curve(p,l) for p,l in zip(adjustments(seq,active,c)[band-1],LENGTHS[seq])]
                    prior=fragment[c][band-1]
                    for j in range(512):
                        if seq==0:g=curves[0][0]*prior[j] if j<256 else curves[0][1][j-256]
                        elif seq==1:g=curves[0][0]*curves[1][0]*prior[j] if j<256 else curves[1][0]*curves[0][1][j-256] if j<368 else curves[1][1][j-368] if j<400 else 1.
                        elif seq==3:g=1. if j<112 else curves[0][0]*curves[1][0]*prior[j-112] if j<144 else curves[1][0]*curves[0][1][j-144] if j<256 else curves[1][1][j-256]
                        else:
                            w,pos=divmod(j,64);g=curves[w][0]*(prior[pos] if w==0 else curves[w-1][1][pos]) if pos<32 else curves[w][1][pos-32]
                        block[j]/=g
                    fragment[c][band-1]=curves[-1][1]
                if seq==0:v=[overlap[c][band][j]+block[j] for j in range(256)];tail=block[256:]
                elif seq==1:v=[overlap[c][band][j]+block[j] for j in range(256)]+block[256:368];tail=block[368:400]
                elif seq==3:v=[overlap[c][band][j]+block[112+j] for j in range(32)]+block[144:256];tail=block[256:]
                else:v=[(overlap[c][band][j] if w==0 else block[64*(w-1)+32+j])+block[64*w+j] for w in range(8) for j in range(32)];tail=block[480:]
                overlap[c][band]=tail;raw.append(v)
            start=4*len(band_history[c]);band_history[c].extend(zip(*raw));signal=[]
            for n in range(start,start+SAMPLES[seq]):
                value=0.
                for b in range(4):
                    for j,q in enumerate(Q):
                        if n>=j and (n-j)%4==0:value+=q*math.cos((2*b+1)*(2*j-3)*math.pi/16)*band_history[c][(n-j)//4][b]
                signal.append(value/65536)
            rows.append(signal)
        offset=len(out)
        for values in zip(*rows):
            for v in values:out.extend(struct.pack('<f',v))
        frames.append(dict(pcm_offset=offset,samples=SAMPLES[seq]))
        previous_shape=shape
    return out,frames

def main():
    blob=bytearray();gold=bytearray();cases=[];invalid=[]
    for channels in [1,2]:
        for mixed in [False,True]:
            for active in [False,True]:
                shapes=[int(mixed and i%2==0) for i in range(6)]
                pcm,frames=oracle(shapes,channels,active);off=len(gold);gold.extend(pcm)
                for i,(seq,shape) in enumerate(zip(SEQUENCES,shapes)):
                    data=packet(i,seq,shape,channels,active);frames[i].update(offset=len(blob),bytes=len(data),sequence=seq,shape=shape);blob.extend(data)
                asc=packed(field(3,5)+frequency(24000)+field(channels,4)+'000')
                case=dict(slots=16,bands=32,channels=channels,asc=asc.hex(),frames=frames,pcm_offset=off,pcm_bytes=len(pcm),samples=len(pcm)//(4*channels),container_rate=24000,container_frame_samples=1472,durations=[SAMPLES[s] for s in SEQUENCES])
                case['video']=video_fixture([case],blob,channels=channels,filename=f'aac-ssr-{channels}-{int(mixed)}-{int(active)}-synthetic.mp4')
                cases.append(case)
    base=next(c for c in cases if c['channels']==1 and c['video']['file']=='aac-ssr-1-0-1-synthetic.mp4')
    frames=[]
    for i,seq in enumerate(SEQUENCES):
        data=packet(i,seq,0,1,True,bad=i==1)
        frames.append(dict(offset=len(blob),bytes=len(data),sequence=seq,shape=0,samples=SAMPLES[seq]));blob.extend(data)
    bad=dict(base,frames=frames)
    bad['video']=video_fixture([bad],blob,channels=1,filename='aac-ssr-invalid-gain-location-synthetic.mp4')
    bad['baseline']=base['video'];bad['error']='AAC SSR invalid gain adjustment location or level'
    invalid.append(bad)
    (DEST/'aac-ssr-packets.bin').write_bytes(blob);(DEST/'aac-ssr-pcm.f32le').write_bytes(gold)
    (DEST/'aac-ssr.json').write_text(json.dumps(dict(cases=cases,invalid=invalid,packet_sha256=hashlib.sha256(blob).hexdigest(),pcm_sha256=hashlib.sha256(gold).hexdigest(),oracle='direct IMDCT, scalar gain/overlap, literal upsampling and convolution'),indent=2)+'\n')
    print('SSR acceptance videos:',len(cases))
if __name__=='__main__':main()
