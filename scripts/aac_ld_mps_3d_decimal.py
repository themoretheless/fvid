"""Independent80-digit polar-factor reference for signalled3D regularization.
No runtime dependency;explicit fixture/reference generation only.
"""
from decimal import Decimal as D,localcontext
from functools import lru_cache
Z=(D(0),D(0));ONE=(D(1),D(0))
def add(a,b):return(a[0]+b[0],a[1]+b[1])
def neg(a):return(-a[0],-a[1])
def sub(a,b):return add(a,neg(b))
def mul(a,b):return(a[0]*b[0]-a[1]*b[1],a[0]*b[1]+a[1]*b[0])
def scale(a,s):return(a[0]*s,a[1]*s)
def conj(a):return(a[0],-a[1])
def absolute(a):return(a[0]*a[0]+a[1]*a[1]).sqrt()
def adjoint(a):return[[conj(a[c][r])for c in range(2)]for r in range(2)]
def product(a,b):return[[add(mul(a[r][0],b[0][c]),mul(a[r][1],b[1][c]))for c in range(2)]for r in range(2)]
def reciprocal(a):
 d=sub(mul(a[0][0],a[1][1]),mul(a[0][1],a[1][0]));n=d[0]*d[0]+d[1]*d[1];inv=scale(conj(d),1/n)
 return[[mul(a[1][1],inv),neg(mul(a[0][1],inv))],[neg(mul(a[1][0],inv)),mul(a[0][0],inv)]]
@lru_cache(None)
def pi():
 def atan(x):
  term=x;total=x;n=1
  while True:
   term*=-x*x;part=term/D(2*n+1);total+=part
   if abs(part)<D('1e-85'):return total
   n+=1
 return 16*atan(D(1)/5)-4*atan(D(1)/239)
def polar(r,p):
 p=(p+pi())%(2*pi())-pi();sin=p;cos=D(1);st=p;ct=D(1);n=1
 while True:
  st*=-p*p/D((2*n)*(2*n+1));ct*=-p*p/D((2*n-1)*(2*n));sin+=st;cos+=ct
  if max(abs(st),abs(ct))<D('1e-85'):return(r*cos,r*sin)
  n+=1
@lru_cache(None)
def level(i):return D(10)**(D(i-16)/20)
@lru_cache(None)
def responses(indices,cld,surround,band):
 rows=[(level(l),level(r),D(p)*pi()/16)for l,r,p in indices];center=rows[2]
 pc=[polar(center[0],center[2]/2 if band<12 else D(0)),polar(center[1],-center[2]/2 if band<12 else D(0))]
 def side(front,rear,db,right):
  ratio=D(10)**(D(db)/10);w=ratio/(1+ratio);f=rows[front];s=rows[rear];g=D(str(surround));energy=[w*f[e]*f[e]+(1-w)*s[e]*s[e]*g*g for e in range(2)];cross=0 if right else 1
  phase=(w*f[cross]*f[cross]*f[2]+(1-w)*s[cross]*s[cross]*g*g*s[2])/energy[cross]*(1 if right else-1)if band<12 else D(0)
  return[polar(energy[0].sqrt(),phase if right else D(0)),polar(energy[1].sqrt(),D(0)if right else phase)]
 return[side(0,3,cld[0],False),side(1,4,cld[1],True),pc]
def inverse(pre,indices,cld,surround,band):
 # The polar factor avoids reproducing the C SVD's near-zero vector division.
 with localcontext()as ctx:
  ctx.prec=80;basis=responses(indices,cld,surround,band)
  h=[[Z for _ in range(2)]for _ in range(2)]
  for ear in range(2):
   for col in range(2):
    for row in range(3):h[ear][col]=add(h[ear][col],scale(basis[row][ear],D.from_float(pre[row][col])))
  det=sub(mul(h[0][0],h[1][1]),mul(h[0][1],h[1][0]));mag=absolute(det)
  if mag>=D('0.1'):return reciprocal(h),False,False
  p=product(adjoint(h),h);trace=p[0][0][0]+p[1][1][0]
  if trace==0:return[[(D(10).sqrt(),D(0)),Z],[Z,(D(10).sqrt(),D(0))]],True,True
  delta=max(D(0),trace*trace-4*mag*mag).sqrt();s0=((trace+delta)/2).sqrt();s1=mag/s0
  phase=ONE if mag==0 else scale(det,1/mag);t=(trace+2*mag).sqrt()
  # U=(H+phase(det(H))*adj(H^H))/sqrt(trace(H^H H)+2|det(H)|).
  correction=[[conj(h[1][1]),neg(conj(h[1][0]))],[neg(conj(h[0][1])),conj(h[0][0])]]
  u=[[scale(add(h[r][c],mul(phase,correction[r][c])),1/t)for c in range(2)]for r in range(2)]
  floor=D('0.1').sqrt()
  if s0<floor and s1<floor:return[[scale(z,1/floor)for z in row]for row in adjoint(u)],True,mag==0
  target=D('0.1')/s0;alpha=(s0-target)/(s0-s1);beta=s0*(target-s1)/(s0-s1)
  root=[[scale(add(p[r][c],(mag,D(0))if r==c else Z),1/t)for c in range(2)]for r in range(2)]
  repaired=[[add(scale(root[r][c],alpha),(beta,D(0))if r==c else Z)for c in range(2)]for r in range(2)]
  return product(reciprocal(repaired),adjoint(u)),True,mag==0

def validate_reference():
 with localcontext()as ctx:
  ctx.prec=80
  for phase in [0,1,7,33,63]:
   indices=tuple((20-ch,9+ch,phase)for ch in range(5))
   basis=responses(indices,(-35.,16.),1.414213,0)
   for value,angle in [(basis[0][1],-D(phase)*pi()/16),(basis[1][0],D(phase)*pi()/16),(basis[2][0],D(phase)*pi()/32),(basis[2][1],-D(phase)*pi()/32)]:
    expected=polar(D(1),angle);actual=scale(value,1/absolute(value))
    assert max(abs(a-b)for a,b in zip(actual,expected))<D('1e-65')
   high=responses(indices,(-35.,16.),1.414213,12)
   assert all(z[1]==0 for row in high for z in row)
if __name__=='__main__':validate_reference();print('Polar reference phase/weighted-average/cutoff identities pass')
