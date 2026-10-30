"""Independent decimal/fraction oracles for physical preparation and matrix time.
Explicit offline generation; ordinary tests load saved controls and traces only.
"""
import hashlib,json,math
from decimal import Decimal,localcontext,ROUND_FLOOR
from fractions import Fraction
from functools import lru_cache
from pathlib import Path
ROOT=Path(__file__).resolve().parents[1]
OUT=ROOT/'tests/fixtures/playback-errors/aac-ld-mps-prepare'
FRAMES=ROOT/'tests/fixtures/playback-errors/aac-ld-mps-frame'
@lru_cache(None)
def split(index,energy,mode,cld):
    # High-precision probabilities/energies, not the runtime softplus formula.
    with localcontext() as ctx:
        ctx.prec=90;e=Decimal(energy);factor=Decimal(1)
        if mode:factor=min(Decimal(5 if mode==1 else 8),1+max(Decimal(0),-e-1)*Decimal(4 if mode==1 else 7)/Decimal(20 if mode==1 else 24))
        adjusted=max(-15,min(15,int((Decimal(index)*factor+Decimal('15.5')).to_integral_value(rounding=ROUND_FLOOR))-15));level=Decimal(str(cld[adjusted+15]));ratio=Decimal(10)**(level/10);left=ratio/(1+ratio);right=1/(1+ratio)
        return adjusted,str(e+10*left.log10()),str(e+10*right.log10())
def smooth(rows,previous,bands):
    result=[]
    for row in rows:
        mode=row['mode']
        if mode==0:previous=dict(time=256,mask=[False]*bands)
        elif mode in [2,3]:
            if mode==2:mask=[True]*bands
            else:
                stride=row['stride'];widths=[stride]*len(row['groups'])
                for i in range(sum(widths)-bands):widths[i%len(widths)]-=1
                mask=[v for v,n in zip(row['groups'],widths)for _ in range(n)]
            previous=dict(time=[64,128,256,512][row['time']],mask=mask)
        result.append(previous)
    return result,previous
def prepare(c,f,facts,previous):
    parameters=[dict(id=p['id'],kind=p['kind'],start=p['start'],indices=[r[:]for r in p['values']])for p in f['parameters']+[p for e in f['extensions']for p in e['parameters']]]
    mode=c.get('physical_quant',c['variant']%3)
    if mode and c['tree']<2:
        parents=[None,(0,True),(0,False),(1,True),None]if c['tree']==0 else[None,(0,True),None,(1,True),(1,False)];energies={}
        for box,parent in enumerate(parents):
            p=next(p for p in parameters if p['id']==f'ott-cld:{box}')
            for ps,row in enumerate(p['indices']):
                for b,index in enumerate(row):
                    energy=energies[parent[0],ps,b][0 if parent[1]else 1]if parent else'0';adjusted,left,right=split(index,energy,mode,tuple(facts['dequantCLD']));row[b]=adjusted;energies[box,ps,b]=(left,right)
    for p in parameters:
        grid=facts['dequant'+p['kind']];offset={'CLD':15,'ICC':0,'CPC':20}[p['kind']];p['values']=[[grid[v+offset]for v in row]for row in p['indices']]
    controls,previous=smooth(f['smoothing'],previous,c['bands']);slots=f['slots'][:]
    if slots[-1]!=c['frame_slots']-1:
        slots.append(c['frame_slots']-1);controls.append(controls[-1])
        for p in parameters:p['indices'].append(p['indices'][-1][:]);p['values'].append(p['values'][-1][:])
    temporal=f['temporal'].copy()
    if temporal['kind']=='ges':
        with localcontext()as ctx:
            ctx.prec=90;grid=[Decimal('0.5'),Decimal('0.5').sqrt(),Decimal(1),Decimal(2).sqrt(),Decimal(2)]
            temporal=dict(kind='ges',envelopes=[[float(grid[v])for v in row]if row is not None else None for row in temporal['envelopes']])
    return dict(slots=slots,parameters=parameters,smoothing=controls,temporal=temporal),previous
def generate():
    facts=json.loads((ROOT/'tests/fixtures/playback-errors/aac-ld-mps-prepare/facts.json').read_text());m=json.loads((FRAMES/'manifest.json').read_text());cases=[]
    for c in m['cases']:
        v=c['variant'];supported=(v%3==0 or c['tree']<2 or c['tree']==2 and v==1)and not(v==2 and c['tree']<2)
        previous=dict(time=0,mask=[False]*c['bands']);prepared=[]
        if supported:
            for f in c['frames']:
                p,previous=prepare(c,f,facts,previous);prepared.append(p)
        cases.append(dict(name=c['name'],supported=supported,frames=prepared))
    extra_cases=[]
    for c in m['cases']:
        if c['variant']!=2:continue
        model=dict(c,physical_quant=0);previous=dict(time=0,mask=[False]*c['bands']);prepared=[]
        for f in c['frames']:
            p,previous=prepare(model,f,facts,previous);prepared.append(p)
        extra_cases.append(dict(name=c['name']+'-quant0',base_case=c['name'],quantization_override=0,frames=prepared))
    scalar=[]
    cld=tuple(facts['dequantCLD'])
    for mode in range(3):
        for db in ['0','-0.5','-1','-1.00001','-2','-3.5','-8.5','-21','-25','-100','-300','-1000']:
            for index in range(-15,16):
                i,left,right=split(index,db,mode,cld);scalar.append(dict(mode=mode,index=index,input=float(db),adjusted=i,left=float(left),right=float(right)))
    matrix=[]
    for bands in [4,5,7,9,12,15,23]:
      for variant in range(4):
        layout=[1,1,1]if variant==0 else[7,2,3]if variant==3 else[2,2,3];width=math.prod(layout);previous=[Fraction(0)]*(bands*width);sequence=[]
        for fi in range(4):
            time=[1,8,16,32][fi];slots=[0]if time==1 else[2,time-2,time-1]if fi%2 else[time//2-1,time-1];cutoff=variant%bands
            controls=[dict(time=[64,128,256,512][(ps+fi)%4],mask=[(b+ps+fi)%3!=0 for b in range(bands)])for ps in range(len(slots))]
            raw=[[Fraction(((b*width+i+ps*7+fi*11)%41)-20,16)for b in range(bands)for i in range(width)]for ps in range(len(slots))]
            endpoints=[];output=[];last=-1
            for ps,slot in enumerate(slots):
                delta=Fraction(slot-last,controls[ps]['time']);current=[delta*v+(1-delta)*previous[i]if controls[ps]['mask'][i//width]and i//width>=cutoff else v for i,v in enumerate(raw[ps])]
                for ts in range(last+1,slot+1):
                    alpha=Fraction(ts-last,slot-last);output.append([float((1-alpha)*a+alpha*b)for a,b in zip(previous,current)])
                endpoints.append([float(v)for v in current]);previous=current;last=slot
            sequence.append(dict(frame_slots=time,slots=slots,controls=controls,residual_cutoff=cutoff,raw=[[float(x)for x in row]for row in raw],endpoints=endpoints,output=output))
        matrix.append(dict(bands=bands,layout=layout,frames=sequence))
    rates_gains=[]
    for rate in [16000,27712,27713,48000,55425,55426,96000]:
      for surround in range(5):
       for lfe in range(5):
        for downmix in range(8):
            rates_gains.append(dict(rate=rate,surround=surround,lfe=lfe,downmix=downmix,qmf_bands=32 if rate<27713 else 128 if rate>=55426 else 64,gains=[facts['surroundGainTable'][surround],facts['lfeGainTable'][lfe],facts['clipGainTable'][downmix]]))
    inputs={name:hashlib.sha256((FRAMES/name).read_bytes()).hexdigest()for name in ['manifest.json','configs.bin','packets.bin']}
    result=dict(scope='Physical parameters and matrix time components only; no spatial PCM/playback acceptance.',inputs=inputs,cases=cases,scalar=scalar,matrix=matrix,rates_gains=rates_gains,extra_cases=extra_cases)
    (OUT/'manifest.json').write_text(json.dumps(result,separators=(',',':'))+'\n');print(sum(c['supported']for c in cases),'physical configs',len(scalar),'energy points',len(matrix),'matrix sequences')
if __name__=='__main__':generate()
