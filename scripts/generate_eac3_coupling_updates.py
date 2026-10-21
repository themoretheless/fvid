"""Owned E-AC-3 coupling updates, participants, geometry and mode transitions."""
import json
from pathlib import Path
from generate_eac3_playback import BitWriter,patch_crc
from generate_eac3_aht import spectrum
from generate_eac3_bsid import movie
from generate_eac3_enhanced_coupling import SUB,DEFAULT,amplitude
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/eac3-coupling-updates'
COUNTS={2:2,3:3,4:3,5:4,6:4,7:5}
ORDINARY_DEFAULT=[0]*8+[1,0,1,1,0,1,1,1,1,1]
def states(kind,nf,blocks):
 result=[]
 for b in range(blocks):
  stage=b*3//blocks;mode='e';begin=2;members=tuple(range(nf))
  if kind=='e-o'and stage>0:mode='o'
  if kind=='o-e'and stage==0:mode='o'
  if kind=='e-o-e'and stage==1:mode='o'
  if kind=='geometry'and stage==1:begin=4
  if kind=='join':members=(0,1)if stage!=1 else(1,nf-1)
  result.append((mode,begin,members))
 return result
def packet(layout,lfe,converted,kind,explicit,reuse,control=False,stop_override=None):
 nf=COUNTS[layout];blocks=6 if nf==2 else 3;stop=stop_override or(121 if nf==2 else 85)
 schedule=states(kind,nf,blocks);owed=[True]*nf
 def geometry(state):
  mode,code,members=state;lo=code+2;hi=SUB.index(stop);start=SUB[lo];mapping={};band=-1
  for sb in range(lo,hi):
   merge=(not explicit and DEFAULT[sb])if mode=='e'else ORDINARY_DEFAULT[sb-4]
   if sb==lo or not merge:band+=1
   for i in range(SUB[sb],SUB[sb+1]):mapping[i]=band
  return start,band+1,mapping
 def code(ch,band,b):return (7+ch*7+band*3+(5 if kind=='updates'and b>=blocks//2 and ch%2==0 else 0))%32
 w=BitWriter().put(0x0b77,16).put(2 if converted else 0,2).put(0,3).put(2047,11)
 w.put(0,2).put(3 if blocks==6 else 2,2).put(layout,3).put(int(lfe),1).put(16,5).put(31,5).zeros(3)
 if not converted and blocks!=6:w.put(0,1)
 if converted:
  if blocks!=6:w.put(1,1)
  w.put(20,6)
 w.put(0,1)
 if blocks==6:w.put(1,1).put(0,1)
 w.put(0,2).zeros(8)
 w.put(int(not control),1)
 for b in range(1,blocks):
  changed=not control and schedule[b]!=schedule[b-1]
  w.put(int(changed),1)
  if changed:w.put(1,1)
 for b in range(blocks):
  changed=b>0 and schedule[b]!=schedule[b-1]
  fresh=b==0 or(changed and not reuse)
  if not control:w.put(1 if fresh else 0,2)
  for _ in range(nf):w.put(1 if fresh else 0,2)
 if lfe:w.put(1,1).zeros(blocks-1)
 if not converted:
  if blocks==6:w.zeros(5*nf)
  else:w.put(0,1)
 w.put(63,6).put(15,4).put(0,1)
 for b,state in enumerate(schedule):
  mode,begin,members=state;start,bands,mapping=geometry(state)
  changed=b>0 and state!=schedule[b-1];fresh=b==0 or(changed and not reuse)
  w.zeros(2)
  if not control:
   if b==0 or changed:
    w.put(int(mode=='e'),1)
    if layout!=2:
     for ch in range(nf):w.put(int(ch in members),1)
    if mode=='e':
     lo=begin+2;hi=SUB.index(stop);w.put(begin,4).put(hi-7,4).put(int(explicit),1)
     if explicit:w.zeros(max(0,hi-max(9,lo+1)))
    else:
     if layout==2:w.put(0,1)
     w.put((start-37)//12,4).put((stop-37)//12-3,4).put(0,1)
   if mode=='e':w.put(0,1)
   first=True
   for ch in range(nf):
    if ch not in members:owed[ch]=True;continue
    update=owed[ch]or(kind=='updates'and b==blocks//2 and ch%2==0)or changed
    reserved=owed[ch]and not first
    if not owed[ch]:
     w.put(int(update),1)
     if mode=='e'and not first:w.put(0,1)
    if update:
     if mode=='o':w.put(0,2)
     for band in range(bands):
      c=code(ch,band,b)
      if mode=='e':w.put(c,5)
      elif c==31:w.put(15,4).put(0,4)
      elif c==0:w.put(2,4).put(0,4)
      else:w.put(3+(c-1)//4,4).put([11,7,3,0][(c-1)%4],4)
    if mode=='e':
     if reserved:w.zeros(9*(bands-1))
     if not first:w.put(0,1)
    owed[ch]=False;first=False
  if layout==2:
   if b==0:w.zeros(4 if control else(2 if begin==2 else 3))
   else:w.put(0,1)
  for ch in range(nf):
   if fresh and(control or ch not in members):w.put((stop-73)//3,6)
  if not control and fresh:
   w.put(5,4)
   for _ in range((stop-start)//3):w.put(62,7)
  if fresh:
   for ch in range(nf):
    end=stop if control or ch not in members else start
    w.put(10,4)
    for _ in range((end-1)//3):w.put(62,7)
    w.zeros(2)
  if lfe and b==0:w.put(10,4).put(62,7).put(62,7)
  if not converted:w.put(0,1)
  if not control:
   if b==0:w.zeros(6)
   else:w.put(0,1)
  coupled_written=False
  for ch in range(nf):
   end=stop if control or ch not in members else start
   for i in range(end):
    value=spectrum(i,ch)[0]
    if control and ch in members and i>=start:value=spectrum(i,2)[0]*amplitude(code(ch,mapping[i],b))
    w.put(round(value)&65535,16)
   if not control and ch in members and not coupled_written:
    for i in range(start,stop):w.put(spectrum(i,2)[0]&65535,16)
    coupled_written=True
  if lfe:
   for i in range(7):w.put(spectrum(i,1)[0]&65535,16)
 return patch_crc(w.to_bytes(4096))
def generate():
 files={};cases=[]
 for layout,nf in COUNTS.items():
  for lfe in [False,True]:
   for converted in [False,True]:
    for kind in ['updates','e-o','o-e','e-o-e','geometry']+(['join']if nf>2 else[]):
     for explicit in [False,True]:
      for reuse in([False,True]if kind in['e-o','o-e','e-o-e']else[False]):
       for wide in([False,True]if layout==2 and kind in['e-o','o-e','e-o-e']and not explicit else[False]):
        name=f'l{layout}-f{int(lfe)}-t{2 if converted else 0}-{kind}-x{int(explicit)}-r{int(reuse)}'+('-wide'if wide else'')
        raw=packet(layout,lfe,converted,kind,explicit,reuse,stop_override=145 if wide else None);control=packet(layout,lfe,converted,kind,explicit,reuse,True,stop_override=145 if wide else None)
        blocks=6 if nf==2 else 3;channels=nf+int(lfe)
        for suffix,data in[('.ec3',raw*3),('-control.ec3',control*3),('.mkv',movie([raw]*3,blocks,channels)),('-control.mkv',movie([control]*3,blocks,channels))]:files[name+suffix]=data
        cases.append(dict(name=name,file=name+'.ec3',control=name+'-control.ec3',video=name+'.mkv',control_video=name+'-control.mkv',blocks=blocks,channels=channels,layout=layout,lfe=lfe,kind=kind,explicit=explicit,reuse_exponents=reuse,wide=wide,quantization_bound=4*(145 if wide else 121)/(32768*1024),exact=False))
 files['manifest.json']=(json.dumps(dict(cases=cases),indent=2)+'\n').encode();return files
if __name__=='__main__':
 OUT.mkdir(parents=True,exist_ok=True)
 for n,b in generate().items():(OUT/n).write_bytes(b)
