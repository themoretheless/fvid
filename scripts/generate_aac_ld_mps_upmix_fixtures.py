"""Independent high-precision covariance and convolution/ducking oracles.
Explicit generation only. No private media, foreign DSP or subprocess/network.
"""
import json,struct
from decimal import Decimal,localcontext
from functools import lru_cache
from pathlib import Path
ROOT=Path(__file__).resolve().parents[1]
OUT=ROOT/'tests/fixtures/playback-errors/aac-ld-mps-upmix'
D=Decimal
PI=D('3.141592653589793238462643383279502884197169399375105820974944592307816406286208998628034825342117')
def add(a,b):return(a[0]+b[0],a[1]+b[1])
def mul(a,b):return(a[0]*b[0]-a[1]*b[1],a[0]*b[1]+a[1]*b[0])
def conj(a):return(a[0],-a[1])
def sub(a,b):return(a[0]-b[0],a[1]-b[1])
ZERO=(D(0),D(0))
@lru_cache(None)
def phase(x):
    x=(x+PI)%(2*PI)-PI
    if x<-PI:x+=2*PI
    s=term=x;c=cterm=D(1)
    for n in range(1,90):
        term *= -x*x/D((2*n)*(2*n+1));s+=term;cterm *= -x*x/D((2*n-1)*(2*n));c+=cterm
    return(c,s)
@lru_cache(None)
def denominator(seed,order,bin,complex_mode,facts_text):
    facts=json.loads(facts_text);r=facts['lattice_coeff_'+{15:'1',6:'2',3:'3'}[order]][seed];a=[(D(1),D(0))]
    for i,k in enumerate(r):
        k=D(str(k));rotation=phase(D(bin)*D('0.5')*D(str(facts['lattice_deltaPhi'][seed][i])))if complex_mode else(D(1),D(0));reflection=(k*rotation[0],k*rotation[1]);a=[a[0]]+[add(a[j],mul(reflection,conj(a[len(a)-j])))for j in range(1,len(a))]+[reflection]
    return tuple(a)
def generate():
    facts_text=(ROOT/'tests/fixtures/playback-errors/aac-ld-mps-upmix/decor-facts.json').read_text();grids=json.loads((ROOT/'tests/fixtures/playback-errors/aac-ld-mps-prepare/facts.json').read_text());blob=bytearray();cases=[];matrices=[]
    with localcontext()as ctx:
      ctx.prec=80
      for cld in range(-15,16):
       for icc in range(8):
        for residual in [False,True]:
         ratio=D(10)**(D(str(grids['dequantCLD'][cld+15]))/10);pl=ratio/(1+ratio);pr=1/(1+ratio);cross=(pl*pr).sqrt();rho=D(str(grids['dequantICC'][icc]));rho=max(rho,-D(11)/(72*cross))if residual else rho;s=(1+2*rho*cross).sqrt();wet=D(0)if residual else cross*(1-rho*rho).sqrt()/s
         matrix=[[(pl+rho*cross)/s,wet,D(1)if residual else D(0)],[(pr+rho*cross)/s,-wet,-D(1)if residual else D(0)]];matrices.append(dict(cld=cld,icc=icc,residual=residual,rows=[[float(x)for x in r]for r in matrix],gains=[float(pl.sqrt()),float(pr.sqrt())],effective_coherence=float(rho)))
      for bands in [32,64]:
       for seed in range(10):
        for config in range(3):
         for mode in ['real','complex']:
          slots=96;source=[[(D((t*17+b*11)%31-15)/64,D((t*13-b*7)%29-14)/128)if t<32 else ZERO for b in range(bands)]for t in range(slots)];raw=[[ZERO]*bands for _ in range(slots)]
          for bin in range(bands):
           edge={0:[14,23],1:[49,64],2:[14,64]}[config];order,delay=(15,7)if bin<edge[0]else(6,2)if bin<edge[1]else(3,1);a=denominator(seed,order,bin,mode=='complex',facts_text);b=list(map(conj,reversed(a)));filtered=[]
           for t in range(slots):
            value=ZERO
            for j,coefficient in enumerate(b):
             if t-delay-j>=0:value=add(value,mul(coefficient,source[t-delay-j][bin]))
            for j in range(1,len(a)):
             if t-j>=0:value=sub(value,mul(a[j],filtered[t-j]))
            filtered.append(value);raw[t][bin]=value
          direct=[D(0)]*23;diffuse=[D(0)]*23;output=[];mapping=grids['QMF23'][:bands]
          for t in range(slots):
           de=[D(0)]*23;re=[D(0)]*23
           for bin,p in enumerate(mapping):
            x,y=source[t][bin],raw[t][bin];de[p]+=x[0]*x[0]+x[1]*x[1];re[p]+=y[0]*y[0]+y[1]*y[1]
           gain=[]
           for p in range(23):
            direct[p]=D('.8')*direct[p]+D('.2')*de[p];diffuse[p]=D('.8')*diffuse[p]+D('.2')*re[p];d,r=direct[p],diffuse[p];g=D(1)
            if r>D('1.5')*d:g=(D('1.5')*d/(r+D('1e-9'))).sqrt()
            elif d>D('1.5')*r:g=min(D(2),(d/(D('1.5')*r+D('1e-9'))).sqrt())
            gain.append(g)
           output.append([(value[0]*gain[mapping[b]],value[1]*gain[mapping[b]])for b,value in enumerate(raw[t])])
          offsets=[]
          for values in [source,output]:
           offsets.append(len(blob));flat=[float(v)for row in values for x in row for v in x];blob.extend(struct.pack('<'+'d'*len(flat),*flat))
          cases.append(dict(name=f'qmf{bands}-seed{seed}-config{config}-{mode}',bands=bands,seed=seed,configuration=config,mode=mode,slots=slots,input_offset=offsets[0],output_offset=offsets[1]))
    (OUT/'signals.bin').write_bytes(blob);(OUT/'manifest.json').write_text(json.dumps(dict(scope='OTT matrix and normalized complex-QMF decorrelation primitives; no full spatial PCM/movie acceptance.',matrices=matrices,cases=cases),separators=(',',':'))+'\n');print(len(matrices),'OTT matrices,',len(cases),'decorrelation sequences,',len(blob),'signal bytes')
if __name__=='__main__':generate()
