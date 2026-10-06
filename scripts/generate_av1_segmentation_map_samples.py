#!/usr/bin/env python3
"""Owned AV1 map updates, temporal prediction and inherited-map pixel fixtures."""
import argparse,hashlib,json,subprocess,copy
from pathlib import Path
from generate_av1_show_existing_samples import Bits,obu,sequence,webm
from generate_av1_alt_q_samples import key
CDF=[[5622,7893,16093,18233,27809,28373,32533,32768],[14274,18230,22557,24935,29980,30851,32344,32768],[27527,28487,28723,28890,32397,32647,32679,32768]]
def unmap(diff,pred,maxid):
    if not pred:return diff
    if pred>=maxid-1:return maxid-diff-1
    if 2*pred<maxid:
        if diff>2*pred:return diff
    elif diff>2*(maxid-pred-1):return maxid-diff-1
    return pred+(diff+1)//2 if diff&1 else pred-diff//2

def adapt_symbols(symbols,initial=None):
    models=copy.deepcopy(initial or {})
    for model in models.values():model[1]=0 # primary CDF loading resets adaptation counts
    out=[]
    for r in symbols:
        if 'model' not in r:out.append(r);continue
        model=json.dumps(r['model'],separators=(',',':'))
        if model not in models:models[model]=[r['cdf'].copy(),0]
        cdf,count=models[model];n=len(cdf);chosen=r['symbol']
        out.append(dict(symbol=chosen,cdf=cdf.copy()))
        rate=3+int(count>15)+int(count>31)+min(2,n.bit_length()-1)
        for i in range(n-1):cdf[i]=cdf[i]-(cdf[i]>>rate) if i<chosen else cdf[i]+((32768-cdf[i])>>rate)
        models[model][1]=min(32,count+1)
    return out,models

def tile(records,ids,maxid,update,temporal,previous,writer,adapt=False,initial=None):
    symbols=[];grid=[0]*64;index=0;above=[0]*8;left=[0]*8
    for r in records:
        if 'block' not in r:symbols.append(r);continue
        x,y,w,h,skip=r['block'];assert not skip
        pred=min(previous[yy*8+xx] for yy in range(y,y+h) for xx in range(x,x+w))
        if update:
            predicted=bool(temporal) and (temporal==1 or index%2==0)
            if temporal:symbols.append(dict(symbol=int(predicted),cdf=[16384,32768],model=[19,[above[x]+left[y]]]))
            if not predicted:
                u=grid[(y-1)*8+x] if y else None;l=grid[y*8+x-1] if x else None;ul=grid[(y-1)*8+x-1] if x and y else None
                spatial=(l if u is None else u if l is None or ul==u else l) or 0
                ctx=0 if ul is None else 2 if ul==u==l else 1 if ul==u or ul==l or u==l else 0
                wanted=ids[index];diff=next(d for d in range(maxid) if unmap(d,spatial,maxid)==wanted)
                symbols.append(dict(symbol=diff,cdf=CDF[ctx],model=[18,[ctx]]));pred=wanted
        for xx in range(x,x+w):above[xx]=int(update and bool(temporal) and predicted)
        for yy in range(y,y+h):
            left[yy]=int(update and bool(temporal) and predicted)
            for xx in range(x,x+w):grid[yy*8+xx]=pred
        index+=1
    if adapt:symbols,models=adapt_symbols(symbols,initial)
    else:models=copy.deepcopy(initial or {})
    text=''.join(f"{len(r['cdf'])} {r['symbol']} "+' '.join(map(str,r['cdf']))+'\n' for r in symbols)
    data=subprocess.run([str(writer)],input=text.encode(),stdout=subprocess.PIPE,check=True).stdout
    return data,grid if update else previous.copy(),models

def inter(t,tile,maxid,update,temporal,enabled,quant,lf,adapt=False,publish=False):
    b=Bits();b.u(0);b.u(1,2);b.u(1);b.u(0);b.u(int(not adapt));b.u(0);b.u(0,3);b.u(1,8)
    b.u(0,21);b.u(0);b.u(int(t['high_precision_mv']));b.u(0);b.u(0,2);b.u(0)
    if adapt:b.u(int(not publish))
    b.u(1)
    b.u(64,8);b.u(0,4);b.u(int(enabled))
    if enabled:
        b.u(int(update))
        if update:b.u(int(bool(temporal)))
        b.u(1)
        for seg in range(8):
            for feature in range(8):
                active=seg<maxid and (feature==0 or lf and 1<=feature<=4)
                b.u(int(active))
                if active:b.u((seg%3-1)*16 if quant and feature==0 else (seg%3-1)*24 if lf and feature else 0,9 if feature==0 else 7)
    b.u(0);b.u(16 if lf else 0,6);b.u(16 if lf else 0,6)
    if lf:b.u(16,6);b.u(16,6)
    b.u(0,3);b.u(0);b.u(int(t['inter_tx_mode']==2));b.u(0);b.u(0);b.u(0,7)
    return obu(6,b.bytes()+tile)

def mapped_key(t,entropy,maxid,quant,lf,adapt=False,publish=False):
    b=Bits();b.u(0);b.u(0,2);b.u(0);b.u(1);b.u(1);b.u(int(not adapt));b.u(0);b.u(255,8);b.u(0)
    if adapt:b.u(int(not publish))
    b.u(1)
    b.u(64,8);b.u(0,4);b.u(1)
    for seg in range(8):
        for feature in range(8):
            active=seg<maxid and (feature==0 or lf and 1<=feature<=4)
            b.u(int(active))
            if active:b.u((seg%3-1)*16 if quant and feature==0 else (seg%3-1)*24 if lf and feature else 0,9 if feature==0 else 7)
    b.u(0);b.u(16 if lf else 0,6);b.u(16 if lf else 0,6)
    if lf:b.u(16,6);b.u(16,6)
    b.u(0,3);b.u(0);b.u(int(t['key_tx_mode']==2));b.u(0)
    return obu(6,b.bytes()+entropy)

def main():
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('--writer',type=Path,required=True);p.add_argument('--oracle',type=Path,required=True);a=p.parse_args()
    root=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors';owned=json.loads((root/'av1-seg-map-owned-symbols.json').read_text());records=owned['symbols']
    assert hashlib.sha256((root/owned['source_file']).read_bytes()).hexdigest()==owned['source_sha256']
    t=next(t for t in json.loads((root/'av1-alt-q-owned-entropy.json').read_text())['templates'] if t['qindex']==64);fixtures=[]
    for adapt,publish in [(False,False),(True,False),(True,True)]:
        for key_update in [False,True]:
            for maxid in range(1,9):
                for reverse,quant,lf in [(False,False,False),(True,True,False),(False,False,True)]:
                    ids=[(9-i if reverse else i)%maxid for i in range(10)];previous=[0]*64;prefix=key(t);models={}
                    if key_update:
                        key_ids=[i%maxid for i in range(sum('block' in r for r in owned['key_symbols']))]
                        entropy,previous,updated=tile(owned['key_symbols'],key_ids,maxid,True,False,previous,a.writer,adapt)
                        if publish:models=updated
                        prefix=mapped_key(t,entropy,maxid,quant,lf,adapt,publish)
                    data=sequence(False,False,False)+prefix;maps=[previous.copy()]
                    # Explicit map, temporal reuse, unchanged map, disable/reset, re-enable inherited zero map.
                    for update,temporal,enabled in [(True,False,True),(True,1,True),(True,2,True),(False,False,True),(False,False,False),(False,False,True)]:
                        entropy,current,updated=tile(records,ids,maxid,update,temporal,previous,a.writer,adapt,models)
                        if publish:models=updated
                        if not enabled:current=[0]*64
                        data+=inter(t,entropy,maxid,update,temporal,enabled,quant,lf,adapt,publish);maps.append(current);previous=current
                    name=f'av1-seg-map-n{maxid}-q{int(quant)}-lf{int(lf)}'+('-intra' if key_update else '')+('-adapt' if adapt else '')+('-publish' if publish else '');obu_name=name+'.obu';yuv_name=name+'.yuv';webm_name=name+'.webm'
                    (root/obu_name).write_bytes(data);wrapped=webm(data);(root/webm_name).write_bytes(wrapped)
                    subprocess.run([str(a.oracle),str(root/obu_name),'6',str(root/yuv_name)],check=True)
                    reference=(root/yuv_name).read_bytes();assert len(reference)==6*1536
                    fixtures.append(dict(file=obu_name,sha256=hashlib.sha256(data).hexdigest(),reference=yuv_name,reference_sha256=hashlib.sha256(reference).hexdigest(),webm=webm_name,webm_sha256=hashlib.sha256(wrapped).hexdigest(),maps=maps,adaptive=adapt,publish=publish))
    (root/'av1-seg-map-generated.json').write_text(json.dumps(dict(fixtures=fixtures),indent=2)+'\n')
if __name__=='__main__':main()
