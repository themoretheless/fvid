"""Independent polynomial SRCPC chains and occupied-column RS stage controls."""
import json
from pathlib import Path
from generate_aac_ep_srcpc_fixtures import MASKS
ROOT=Path(__file__).resolve().parents[1]
OUT=ROOT/'tests/fixtures/playback-errors/aac-ep-class-codes'
def chain(messages,rates,terminated):
    history=[];phase=0;active=0;result=[]
    for message,rate,term in zip(messages,rates,terminated):
        if not message and not term:result.append('');continue
        coded=''
        def at(t):return history[t]if t>=0 else 0
        for i in range(len(message)+4*term):
            t=len(history);feedback=at(t-1)^at(t-2)^at(t-4);u=int(message[i])if i<len(message)else feedback
            history.append(u^feedback)
            values=[u,at(t)^at(t-3)^at(t-4),at(t)^at(t-2)^at(t-3)^at(t-4),at(t)^at(t-1)^at(t-3)^at(t-4)]
            if phase==0:active=rate
            coded+=''.join(str(v)for v,mask in zip(values,MASKS[active])if mask&(128>>phase));phase=(phase+1)%8
        if term:assert history[-4:]==[0]*4;phase=0
        result.append(coded)
    return result

def octet_stage(x,y):
    if not x:return list(y)
    assert len(x)%8==0
    width=len(x)//8;total=len(x)+len(y)
    rows=[[None]*min(width,total-i)for i in range(0,total,width)]
    for i,value in enumerate(x):rows[i%8][i//8]=value
    it=iter(y)
    for col in range(width):
        for row in rows:
            if col<len(row)and row[col]is None:row[col]=next(it)
    assert next(it,None)is None
    return [row[col]for col in range(width)for row in rows if col<len(row)]
def generate():
    chains=[];rows=[];blob=bytearray()
    for rate in range(25):
        for lengths in [[8,16,24],[7,9,4],[0,8,0],[31,1,8]]:
            for terms in [[False,False,True],[True,False,True]]:
                messages=[''.join(str((j*j+rate+j//3+i)%2)for j in range(n))for i,n in enumerate(lengths)]
                rates=[rate]*3;encoded=chain(messages,rates,terms)
                chains.append(dict(messages=messages,rates=rates,terminated=terms,encoded=encoded))
        for terms in [[False,False,True],[True,False,True]]:
            messages=[''.join(str((j+rate+i*j//3)%2)for j in range(n))for i,n in enumerate([8,16,24])]
            rates=[rate,(rate+7)%25,(rate+13)%25];encoded=chain(messages,rates,terms)
            chains.append(dict(messages=messages,rates=rates,terminated=terms,encoded=encoded))
    for rate in range(25):
        for lengths in [[7,9,4],[3,3,10],[0,1,15]]:
            messages=[''.join(str((j//3+i+rate)%2)for j in range(n))for i,n in enumerate(lengths)]
            rates=[rate,(rate+7)%25,(rate+13)%25];terms=[False,False,True]
            chains.append(dict(messages=messages,rates=rates,terminated=terms,encoded=chain(messages,rates,terms)))
    messages=['0101101','','1'];rates=[1,24,1];terms=[False,False,True]
    chains.append(dict(messages=messages,rates=rates,terminated=terms,encoded=chain(messages,rates,terms)))
    messages=['01011010','1001010011000110'];rates=[1,24];terms=[False,True];encoded=chain(messages,rates,terms);damaged=list(encoded);damaged[0]=damaged[0][:-1]+str(1-int(damaged[0][-1]))
    # Exhaustively establish a unique full-chain minimum; this is stronger
    # than assuming that a later section can correct every unprotected bit.
    observed=''.join(damaged);distances=[]
    for candidate in range(256):
        word=f'{candidate:08b}';value=''.join(chain([word,messages[1]],rates,terms))
        distances.append(sum(a!=b for a,b in zip(value,observed)))
    assert min(distances)==1 and distances.count(1)==1 and distances.index(1)==int(messages[0],2)
    chains.append(dict(messages=messages,rates=rates,terminated=terms,encoded=encoded,damaged=damaged))
    for width in range(1,34):
        for ly in list(range(34))+[63,64,65,127,128,129,255]:
            x=bytes((i*37+width)%256 for i in range(8*width));y=bytes((i*71+width*3)%256 for i in range(ly));z=bytes(octet_stage(x,y))
            rows.append(dict(x=len(x),y=len(y),offset=len(blob)));blob.extend(x+y+z)
    return {'vectors.bin':bytes(blob),'manifest.json':(json.dumps(dict(chains=chains,octet_stages=rows,provenance='Own polynomial feedback/convolution over message history and independent occupied-cell octet-column matrix. No external codec, process, network or private media.'),indent=2)+'\n').encode()}
if __name__=='__main__':
    OUT.mkdir(exist_ok=True)
    for name,data in generate().items():(OUT/name).write_bytes(data)
