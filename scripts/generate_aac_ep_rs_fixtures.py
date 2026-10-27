"""Own MPEG-4 SRS arithmetic/shortening/layout oracles; offline explicit only."""
import json,functools
from pathlib import Path
ROOT=Path(__file__).resolve().parents[1]
OUT=ROOT/'tests/fixtures/playback-errors/aac-ep-rs'
def mul(a,b):
    value=0
    for bit in range(8):
        if b>>bit&1:value^=a<<bit
    while value.bit_length()>8:value^=0x11d<<(value.bit_length()-9)
    return value
@functools.lru_cache(None)
def generator(k):
    polynomial=[1];root=1
    for _ in range(2*k):
        root=mul(root,2);next=[0]*(len(polynomial)+1)
        for i,value in enumerate(polynomial):next[i]^=mul(value,root);next[i+1]^=value
        polynomial=next
    return polynomial
# Independent ascending-coefficient polynomial long division; no production LUT.
def parity(data,k):
    r=2*k;g=generator(k);work=[0]*r+list(data)
    for degree in range(len(work)-1,r-1,-1):
        coefficient=work[degree]
        for i,v in enumerate(g):work[degree-r+i]^=mul(coefficient,v)
    return bytes(work[:r])
def encode(data,k):
    capacity=255-2*k
    return data+b''.join(parity(data[i:i+capacity],k)for i in range(0,len(data),capacity))
def corrupt(encoded,n,k):
    wire=bytearray(encoded);capacity=255-2*k;trailer=n;corrected=0
    for a in range(0,n,capacity):
        size=min(capacity,n-a);positions=list(range(a,a+size))+list(range(trailer,trailer+2*k))
        for j in range(k):wire[positions[j*len(positions)//k]]^=(17*j+91+k)%255+1
        trailer+=2*k;corrected+=k
    return bytes(wire),corrected

def generate():
    vectors=[]
    for k in range(128):
        capacity=255-2*k
        for n in sorted({0,1,2,max(0,capacity-1),capacity,capacity+1,2*capacity,2*capacity+1}):
            data=bytes((i*73+i*i*11+k*7)%256 for i in range(n));encoded=encode(data,k);damaged,count=corrupt(encoded,n,k)
            vectors.append(dict(k=k,data=data.hex(),encoded=encoded.hex(),damaged=damaged.hex(),corrected=count))
    manifest=dict(vectors=vectors,provenance='Own carryless GF multiplication with polynomial 0x11d; ascending polynomial long division, roots alpha^1..alpha^(2k), low coefficient first, all parity trailers after class. No private media, foreign encoder/decoder, process or network.')
    return {'manifest.json':(json.dumps(manifest,indent=2)+'\n').encode(),'gf-products.bin':bytes(mul(a,b)for a in range(256)for b in range(256))}
if __name__=='__main__':
    OUT.mkdir(exist_ok=True)
    for name,data in generate().items():(OUT/name).write_bytes(data)
