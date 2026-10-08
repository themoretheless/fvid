#!/usr/bin/env python3
"""Explicit offline Decimal/algebraic PS mixing oracle, no codec or FFmpeg."""
import json
from itertools import product
from decimal import Decimal as D, getcontext
from pathlib import Path
getcontext().prec=90
ROOT=Path(__file__).resolve().parents[1]
PI=D('3.141592653589793238462643383279502884197169399375105820974944592307816406286208998628034825342117')
SQRT2=D(2).sqrt()
def atan(x):
    sign=-1 if x<0 else 1;x=abs(x);scale=1
    while x>D('.1'):x=x/(1+(1+x*x).sqrt());scale*=2
    term=x;total=x;n=1
    while True:
        term*=-x*x;n+=2;new=total+term/n
        if new==total:return sign*scale*new
        total=new
def sincos(x):
    s=x;c=D(1);st=x;ct=D(1);n=1
    while True:
        st*=-x*x/((2*n)*(2*n+1));ct*=-x*x/((2*n-1)*(2*n));ns=s+st;nc=c+ct
        if ns==s and nc==c:return s,c
        s,c=ns,nc;n+=1
def matrix(db,rho,mode):
    c=D(10)**(D(db)/20)
    if mode=='a':
        alpha=PI/2 if rho==-1 else atan(((1-rho)/(1+rho)).sqrt())
        c1=SQRT2/(1+c*c).sqrt();c2=c*c1;beta=alpha*(c1-c2)/SQRT2
        sa,ca=sincos(alpha+beta);sb,cb=sincos(beta-alpha)
        return [ca*c2,cb*c1,sa*c2,sb*c1]
    rho=max(rho,D('.05'))
    # Independent algebraic eigenvector: avoids production atan/modulo path.
    den=((c*c-1)**2+4*c*c*rho*rho).sqrt()
    ca=((1+(c*c-1)/den)/2).sqrt();sa=((1-(c*c-1)/den)/2).sqrt()
    mu=1+(4*rho*rho-4)/(c+1/c)**2
    cg=((1+mu.sqrt())/2).sqrt();sg=((1-mu.sqrt())/2).sqrt()
    return [SQRT2*ca*cg,SQRT2*sa*cg,-SQRT2*sa*sg,SQRT2*ca*sg]
def rotation(indices):
    re=im=D(0)
    for weight,index in zip([D('.25'),D('.5'),D(1)],indices):
        sn,cs=sincos(PI*index/4);re+=weight*cs;im+=weight*sn
    length=(re*re+im*im).sqrt()
    return re/length,im/length
def phase_coefficients(real,ipd,opd):
    ir,ii=rotation(ipd);o_r,o_i=rotation(opd)
    right=(o_r*ir+o_i*ii,o_i*ir-o_r*ii)
    return [(h*re,h*im) for h,(re,im) in zip(real,[(o_r,o_i),right,(o_r,o_i),right])]
def main():
    saved=json.loads((ROOT/'tests/fixtures/playback-errors/aac-ps-dequant-oracles.json').read_text())
    rows=[]
    for fine,key in [(False,'iid_coarse'),(True,'iid_fine')]:
        for iid in saved[key]:
            for icc in saved['icc']:
                for mode in ['a','b']:
                    rows.append(dict(fine=fine,iid=iid['index'],icc=icc['index'],mode=mode,expected=[str(x) for x in matrix(iid['db'],D(icc['value']),mode)]))
    phase=[]
    base=matrix('4',D('.36764'),'a')
    for values in product(range(8),repeat=3):
        for family in ['ipd','opd']:
            ipd=values if family=='ipd' else (7,0,1)
            opd=values if family=='opd' else (0,7,2)
            ir,ii=rotation(ipd);o_r,o_i=rotation(opd)
            right=(o_r*ir+o_i*ii,o_i*ir-o_r*ii)
            expected=[]
            for h,(re,im) in zip(base,[(o_r,o_i),right,(o_r,o_i),right]):expected.append([str(h*re),str(h*im)])
            # Original complex signal pair, deliberately unequal real/imag parts.
            h=[(D(re),D(im)) for re,im in expected]
            def mul(a,b):return a[0]*b[0]-a[1]*b[1],a[0]*b[1]+a[1]*b[0]
            source=(D('.3'),D('-.7'));decor=(D('.4'),D('.2'))
            outputs=[]
            for direct,diffuse in [(0,2),(1,3)]:
                a=mul(h[direct],source);b=mul(h[diffuse],decor)
                outputs.append([str(a[0]+b[0]),str(a[1]+b[1])])
            phase.append(dict(ipd=ipd,opd=opd,expected=expected,outputs=outputs))
    (ROOT/'tests/fixtures/playback-errors/aac-ps-mixing-oracles.json').write_text(json.dumps(dict(kind='original Decimal PS matrix and phase oracle',rows=rows,phase=phase),separators=(',',':'))+'\n')
    print(len(rows),'matrix cases')
if __name__=='__main__':main()
