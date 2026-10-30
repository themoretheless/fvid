"""Own inverse-coded CLD/ICC/CPC pair vectors, independent of decoder code.
Generation is explicit/offline; saved codeword facts have official provenance.
"""
import json
from pathlib import Path
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/aac-ld-mps-entropy'
def field(x,n):assert 0<=x<1<<n;return format(x,f'0{n}b')if n else''
def packed(s):return int(s+'0'*(-len(s)%8),2).to_bytes((len(s)+7)//8,'big')
def quant(kind,coarse):return {'CLD':(15,7,0)if coarse else(31,15,0),'ICC':(4,0,0)if coarse else(8,0,0),'CPC':(26,10,0)if coarse else(51,20,1)}[kind]
def pcm(values,levels):
 group={3:5,7:6,11:2,13:4,19:4,25:3,51:4,4:1,8:1,15:1,26:1,31:1}[levels];wire=''
 for i in range(0,len(values),group):
  row=values[i:i+group];value=0
  for v in row:assert 0<=v<levels;value=value*levels+v
  wire+=field(value,(levels**len(row)-1).bit_length())
 return wire
class Writer:
 def __init__(self,books):self.books=books;self.wire='';self.escapes=[]
 def code(self,book,symbol):
  row=next(r for r in self.books[book]if r[2]==symbol);self.wire+=field(row[1],row[0])
 def signed(self,kind,time,v):self.code(kind+('_T1'if time else'_F1'),abs(v));self.wire+=str(int(v<0))if v else''
 def vector(self,kind,time,values,lav_index,escape):
  if lav_index is None:
   start=0
   if not time:self.code(kind+'_P0',values[0]);start=1
   for v in values[start:]:self.signed(kind,time,v)
   self.escapes.append(0);return
  lav={'CLD':[3,5,7,9],'ICC':[1,3,5,7],'CPC':[3,6,9,12]}[kind][lav_index]
  book=kind+('_T2_'if time else'_F2_')+str(lav_index);self.code('LAV',lav_index);start=0
  if not time:self.code(kind+'_P0',values[0]);start=1
  pairs=[];i=start
  while i+1<len(values):pairs.append(values[i:i+2]);i+=2
  escapes=[]
  for target in pairs:
   if escape and any(r[2]==-1 for r in self.books[book]):
    self.code(book,-1);escapes.extend(v+lav for v in target);continue
   found=None
   for _,_,raw in self.books[book]:
    if raw<0:continue
    a,b=raw>>4,raw&15
    base=[2*lav+1-a-b,b-a]if a+b>lav else[a+b,a-b]
    for negative in [False,True]if sum(base)!=0 else[False]:
     signed=[-v for v in base]if negative else base[:]
     for swap in [False,True]if signed[0]!=signed[1]else[False]:
      out=signed[::-1]if swap else signed
      if out==target:found=(raw,(str(int(negative))if sum(base)!=0 else'')+(str(int(swap))if base[0]!=base[1]else''));break
     if found:break
    if found:break
   if found:self.code(book,found[0]);self.wire+=found[1]
   else:
    assert any(r[2]==-1 for r in self.books[book]),(book,target)
    self.code(book,-1);escapes.extend(v+lav for v in target)
  self.wire+=pcm(escapes,2*lav+1)if escapes else'';self.escapes.append(len(escapes)//2)
  if i<len(values):self.signed(kind,time,values[i])
def generate():
 books=json.loads((OUT/'codewords.json').read_text());blob=bytearray();cases=[]
 for kind in ['CLD','ICC','CPC']:
  for coarse in [False,True]:
   levels,offset,lsb=quant(kind,coarse)
   for n in [1,2,3,4,7,15,23]:
    for paired in [False,True]:
     contexts= [('independent',True,0,False,False),('dependent-freq',False,0,False,False),('history',False,0,True,False),('second-freq',True,1,False,False),('second-set',True,1,True,False)]if not paired else[('independent',True,0,False,False),('independent-freq-time',True,0,False,True),('independent-time-book',True,0,True,False),('dependent-freq-freq',False,0,False,False),('history',False,0,True,True),('freq-time',False,0,False,True),('time-freq',False,0,True,False)]
     for context,independent,set_index,time0,time1 in contexts:
      allow=not independent or set_index!=0;history=[((i*3+2)%(min(levels,5)))-min(offset,2)for i in range(n)]
      if kind=='ICC':history=[(i%2)+1-(i%3==0)for i in range(n)]
      center=offset if kind!='ICC' else 1
      msb=[[((center+((i+row)%2))>>lsb)for i in range(n)]for row in range(1+paired)]
      if independent and time0 and not allow:msb[0]=[1+i%2 for i in range(n)]
      values=[[((v<<lsb)+((i+row)%2 if lsb else 0))-offset for i,v in enumerate(vector)]for row,vector in enumerate(msb)]
      # Difference vectors are derived from the desired indices, including the
      # independent first pair's time-context / frequency-reconstruction rule.
      delta=[]
      for row,vector in enumerate(msb):
       time=[time0,time1][row]
       if not time or row==0 and not allow:d=[vector[0]]+[b-a for a,b in zip(vector,vector[1:])]
       else:
        prior=[(v+offset)>>lsb for v in history]if row==0 else msb[0];d=[v-p for v,p in zip(vector,prior)]
       delta.append(d)
      modes=[('pcm',None,False),('1d',None,False)]+[('2d',lav,esc)for lav in range(4)for esc in [False,True]]
      for scheme,lav,esc in modes:
       if esc and not any(any(r[2]==-1 for r in books[kind+('_T2_'if time else'_F2_')+str(lav)])for time in [time0,time1][:1+paired]):continue
       w=Writer(books);w.wire='1'if scheme=='pcm'else'0'
       if scheme=='pcm':w.wire+=pcm([values[row][i]+offset for i in range(n)for row in range(1+paired)],levels)
       else:
        if paired or allow:w.wire+=str(int(time0))
        if paired and (not time0 or allow):w.wire+=str(int(time1))
        w.wire+=str(int(scheme=='2d'))
        for row in range(1+paired):w.vector(kind,[time0,time1][row],delta[row],lav,esc)
        if lsb:w.wire+=''.join(str((values[row][i]+offset)&1)for row in range(1+paired)for i in range(n))
       packet=packed(w.wire);bug=kind=='CLD'and any(count>2 for count in w.escapes)
       name=f'{kind}-q{int(coarse)}-n{n}-p{int(paired)}-{context}-{scheme}-l{lav}-e{int(esc)}'
       cases.append(dict(name=name,kind=kind,coarse=coarse,paired=paired,independent=independent,set_index=set_index,history=history,values=values,scheme=scheme,lav=lav,escape_counts=w.escapes,reference_escape_overlap=bug,bits=len(w.wire),offset=len(blob),bytes=len(packet)));blob.extend(packet)
 # Exhaust the signed symmetry domain of every LAV, without requiring a
 # frequency cumulative vector to remain in range. A distinct valid history
 # value for each band admits both positive and negative time differences.
 for kind in ['CLD','ICC','CPC']:
  for coarse in [False,True]:
   levels,offset,lsb=quant(kind,coarse);limit=(levels-1)>>lsb
   for lav_index,lav in enumerate({'CLD':[3,5,7,9],'ICC':[1,3,5,7],'CPC':[3,6,9,12]}[kind]):
    for x in range(-min(lav,limit),min(lav,limit)+1):
     for y in range(-min(lav,limit),min(lav,limit)+1):
      delta=[x,y];prior=[limit if d<0 else 0 for d in delta];current=[p+d for p,d in zip(prior,delta)]
      history=[(v<<lsb)-offset for v in prior];values=[(v<<lsb)+(int(i%2==1 and v<limit)if lsb else 0)-offset for i,v in enumerate(current)]
      w=Writer(books);w.wire='011';w.vector(kind,True,delta,lav_index,False)
      if lsb:w.wire+=''.join(str((v+offset)&1)for v in values)
      packet=packed(w.wire);name=f'{kind}-q{int(coarse)}-symmetry-l{lav_index}-x{x}-y{y}'
      cases.append(dict(name=name,kind=kind,coarse=coarse,paired=False,independent=False,set_index=0,history=history,values=[values],scheme='2d',lav=lav_index,escape_counts=w.escapes,reference_escape_overlap=False,bits=len(w.wire),offset=len(blob),bytes=len(packet)));blob.extend(packet)
 # A zero-band subregion has a valid empty PCM representation. Its single
 # coding flag is still consumed, and a paired request retains two empty rows.
 for kind in ['CLD','ICC','CPC']:
  for coarse in [False,True]:
   for paired in [False,True]:
    for independent,set_index in [(True,0),(False,0)]+([]if paired else[(True,1)]):
     cases.append(dict(name=f'{kind}-q{int(coarse)}-empty-pcm-p{int(paired)}-i{int(independent)}-s{set_index}',kind=kind,coarse=coarse,paired=paired,independent=independent,set_index=set_index,history=[],values=[[]for _ in range(1+paired)],scheme='pcm',lav=None,escape_counts=[],reference_escape_overlap=False,bits=1,offset=len(blob),bytes=1));blob.extend(b'\x80')
 files={'packets.bin':bytes(blob),'manifest.json':(json.dumps(dict(cases=cases,provenance='Own expected indices and inverse coding. Numeric prefix facts frozen separately. Entropy component only, no spatial movie playback acceptance.'),indent=2)+'\n').encode()};return files
if __name__=='__main__':
 for name,data in generate().items():(OUT/name).write_bytes(data)
