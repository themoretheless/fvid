"""Quiet-threshold BAP authoring with an independent high-precision control."""
import json,math,struct
from pathlib import Path
from generate_eac3_playback import BitWriter,patch_crc
from generate_eac3_bsid import movie
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/eac3-reduced-allocation'
TABLES=json.loads(Path(__file__).with_name('dolby_reduced_rate_tables.json').read_bytes())
BITS={6:5,7:6,8:7,9:8,10:9,11:10,12:11,13:12,14:14,15:16}
EXP=[15,17,19,21,23]+[24]*68

def bap(rate,shift,snr):
    # With DBKNEE=0, fastgain>=640 and this falling/quiet spectrum,
    # excitation is below every quiet threshold. Thus mask=HTH exactly;
    # no leak-recursion implementation is used to author the expected widths.
    result=[]
    for i,e in enumerate(EXP):
        quiet=TABLES['HTH'][rate][TABLES['MASKTAB'][i]>>shift]
        mask=max(0,quiet-snr+2048)&0x1fe0
        address=max(0,min(63,((3072-(e<<7))-(mask-2048))>>5))
        result.append(TABLES['BAPTAB'][address])
    return result

def quant(value,code):
    if code==0:return 0,0.
    if code in [1,2,3,4,5]:
        levels={1:3,2:5,3:7,4:11,5:15}[code]
        q=max(0,min(levels-1,round((value*levels+levels-1)/2)))
        return q,(2*q-levels+1)/levels
    bits=BITS[code];q=max(-(1<<(bits-1)),min((1<<(bits-1))-1,round(value*(1<<(bits-1)))))
    return q,q/(1<<(bits-1))

def mantissas(w,codes,values):
    encoded=[quant(v,c)for c,v in zip(codes,values)];groups={}
    for code,levels,size in [(1,3,3),(2,5,3),(4,11,2)]:
        indices=[i for i,c in enumerate(codes)if c==code]
        for j in range(0,len(indices),size):
            selected=indices[j:j+size];digits=[encoded[i][0]for i in selected]+[(levels-1)//2]*(size-len(selected))
            number=0
            for d in digits:number=number*levels+d
            groups[selected[0]]=(number,5 if code==1 else 7)
    for i,(code,(q,_))in enumerate(zip(codes,encoded)):
        if i in groups:w.put(*groups[i])
        elif code in [3,5]:w.put(q,3 if code==3 else 4)
        elif code>=6:w.put(q&((1<<BITS[code])-1),BITS[code])
    return [v for _,v in encoded]

def packet(rate,reduced,snr,values):
    w=BitWriter().put(0x0b77,16).put(0,2).put(0,3).put(511,11)
    w.put(3 if reduced else rate,2).put(rate if reduced else 3,2).put(1,3).put(0,1).put(16,5)
    w.put(31,5).zeros(4) # compre,mixmdate,infomdate,addbsie
    w.put(1,1).put(0,1).put(0,2).put(0,1).put(0,1).put(1,1).put(1,1).zeros(4)
    for block in range(6):w.put(1 if block==0 else 0,2)
    w.zeros(5).put((snr//4)//16+15,6).put((snr//4)%16,4).put(0,1)
    decoded=[];codes=bap(rate,int(reduced),snr)
    for block in range(6):
        w.zeros(3) # dithflag,dynrnge,spxinu
        if block==0:
            w.put(0,6).put(15,4).put(124,7).put(117,7)
            for _ in range(22):w.put(62,7)
            w.zeros(2)
        w.put(int(block==0),1) # baie
        if block==0:w.put(3,2).put(3,2).put(3,2).put(0,2).put(7,3)
        w.put(0,1) # convsnroffste follows bit allocation parameters
        decoded.append(mantissas(w,codes,values[block]))
    return patch_crc(w.to_bytes(1024)),decoded,codes

def generate():
    files={};cases=[]
    for rate_index,rate in enumerate([24000,22050,16000]):
        codes=bap(rate_index,1,1088)
        values=[[0. if i<8 else .5*((i+b)%3-1)for i in range(73)]for b in range(6)]
        raw,decoded,codes=packet(rate_index,True,1088,values)
        control,reference,reference_codes=packet(rate_index,False,3132,decoded)
        error=max(4*sum(abs(a-b)*2**-e for a,b,e in zip(x,y,EXP))for x,y in zip(decoded,reference))
        stem=f'quiet-{rate}';files[stem+'.ec3']=raw*3;files[stem+'-control.ec3']=control*3
        files[stem+'.mkv']=movie([raw]*3,channels=1,rate=rate)
        files[stem+'-control.mkv']=movie([control]*3,channels=1,rate=rate*2)
        cases.append(dict(name=stem,rate=rate,channels=1,error_bound=error,bap=codes,reference_bap=reference_codes,frames=3*1536-256))
    files['manifest.json']=(json.dumps(dict(cases=cases,provenance='Owned mono spectra, grouped quantizers and neutral VP9. Quiet-limit closed-form BAP uses pinned public ATSC tables. PCM control uses separately encoded high-precision full-rate quantizers with analytical L1 error bound. No private media, subprocess, FFmpeg or network.'),indent=2)+'\n').encode()
    return files
if __name__=='__main__':
    OUT.mkdir(parents=True,exist_ok=True)
    for n,b in generate().items():(OUT/n).write_bytes(b)
