#!/usr/bin/env python3
"""Offline 80-digit Decimal TTT and complex-adjugate oracle."""
import json,pathlib,struct
from decimal import Decimal as D,localcontext
ROOT=pathlib.Path(__file__).resolve().parents[1]
OUT=ROOT/'tests/fixtures/playback-errors/aac-ld-mps-ttt'
def sub(a,b):return(a[0]-b[0],a[1]-b[1])
def mul(a,b):return(a[0]*b[0]-a[1]*b[1],a[0]*b[1]+a[1]*b[0])
def div(a,b):
 d=b[0]*b[0]+b[1]*b[1];return((a[0]*b[0]+a[1]*b[1])/d,(a[1]*b[0]-a[0]*b[1])/d)
def transform(mode,x,y,rho,residual):
 root2=D(2).sqrt();third=D(1)/3
 if mode<2:
  k=D(1)if residual else rho
  a=[[(x+2)/3/k,(y-1)/3/k,third],[(x-1)/3/k,(y+2)/3/k,third],[(1-x)/3/k,(1-y)/3/k,-third]]
 else:
  # Independent unnormalized energies C=1, R=a/(1+b), L=a*b/(1+b).
  a=D(10)**(x/10);b=D(10)**(y/10);r=a/(1+b);l=r*b;c=D(1)
  a=[[(l/(l+c)).sqrt(),D(0),third],[D(0),(r/(r+c)).sqrt(),third],[(c/(l+c)).sqrt()/2,(c/(r+c)).sqrt()/2,-third]]
  if mode==2:
   wl=((r+c)/(r+c*(1+r/l))).sqrt();wr=((l+c)/(l+c*(1+l/r))).sqrt()
   a[0][:2]=[wl,-c*wl/(r+c)];a[1][:2]=[-c*wr/(l+c),wr]
 a[2]=[v*root2 for v in a[2]];return [v for row in a for v in row]
def inversion(mode,ott,x,y):
 if mode==0:
  def f(z):return D(1)if abs(z)>=1 else -1-2*z if z<D('-.5')else (1+2*z)/3
  factors=[f(x),f(y)]
 else:
  a=D(10)**(x/10);b=D(10)**(y/10);factors=[(a*b/(a*b+2*(b+1))).sqrt(),(a/(a+2*(b+1))).sqrt()]
 w=[f/(1+D(10)**(d/20))for f,d in zip(factors,ott)]
 a,b=w;na=((1-a)**2+a*a).sqrt();nb=((1-b)**2+b*b).sqrt();s=D(3).sqrt();z=D(0)
 h=[((1-a)/na,a/na),(z,b/nb/s),(z,-a/na/s),((1-b)/nb,-b/nb)]
 det=sub(mul(h[0],h[3]),mul(h[1],h[2]));inv=[div(h[3],det),div((-h[1][0],-h[1][1]),det),div((-h[2][0],-h[2][1]),det),div(h[0],det)]
 return w+[v for pair in inv for v in pair]
def main():
 facts=json.loads((ROOT/'tests/fixtures/playback-errors/aac-ld-mps-prepare/facts.json').read_text(),parse_float=D)
 cld=facts['dequantCLD'];cpc=facts['dequantCPC'];icc=facts['dequantICC'];data=bytearray();cases=[];inversions=[]
 with localcontext()as ctx:
  ctx.prec=80
  for mode,grid,shift in ((0,cpc,20),(2,cld,15),(4,cld,15)):
   for i,x in enumerate(grid):
    for j,y in enumerate(grid):
     for k,rho in enumerate(icc if mode==0 else [D(1)]):
      for residual in ((False,True)if mode==0 else(False,)):
       if mode==0 and rho==0 and not residual:continue
       cases.append([mode,i-shift,j-shift,k,residual,len(data)])
       data.extend(struct.pack('<9d',*[float(v)for v in transform(mode,x,y,rho,residual)]))
  for mode in (0,2):
   grid=cpc if mode==0 else cld;shift=20 if mode==0 else 15
   for i,left in enumerate(cld):
    for j,right in enumerate(cld):
     for trial in range(4):
      a=(i*7+j*3+trial*13)%len(grid);b=(i*11+j*5+trial*17)%len(grid)
      inversions.append([mode,i-15,j-15,a-shift,b-shift,len(data)])
      data.extend(struct.pack('<10d',*[float(v)for v in inversion(mode,[left,right],grid[a],grid[b])]))
 OUT.mkdir(parents=True,exist_ok=True);(OUT/'expected.bin').write_bytes(data);(OUT/'manifest.json').write_text(json.dumps(dict(scope='TTT transform/inversion primitives only; no full spatial movie acceptance.',cases=cases,inversions=inversions),separators=(',',':'))+'\n');print(len(cases),len(inversions),len(data))
if __name__=='__main__':main()
