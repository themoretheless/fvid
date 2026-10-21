"""Owned valid AHT/coupling-update combinations and uncoupled controls."""
import json
from pathlib import Path
from generate_eac3_playback import BitWriter,patch_crc
from generate_eac3_aht import spectrum,conventional
from generate_eac3_bsid import movie
from generate_eac3_coupling_updates import COUNTS,ORDINARY_DEFAULT,states
from generate_eac3_enhanced_coupling import SUB,DEFAULT,amplitude
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/eac3-aht-updates'
def plan(layout,kind):
 nf=COUNTS[layout];schedule=states(kind,nf,6)
 if kind=='join':schedule=[(m,1,p)for m,_,p in schedule]
 def geometry(state):
  mode,begin,members=state;lo=begin*2 if begin<3 else begin+2;start=SUB[lo];mapping={};band=-1
  for sb in range(lo,7):
   merge=DEFAULT[sb]if mode=='e'else ORDINARY_DEFAULT[sb-4]
   if sb==lo or not merge:band+=1
   for i in range(SUB[sb],SUB[sb+1]):mapping[i]=band
  return start,band+1,mapping
 widths=[[geometry(s)[0]if ch in s[2]else 73 for ch in range(nf)]for s in schedule]
 eligible=[ch for ch in range(nf)if len({r[ch]for r in widths})==1]
 if all(s==schedule[0]for s in schedule):eligible.append(5)
 return schedule,geometry,widths,eligible
def packet(layout,lfe,converted,kind,active,basis,control_offset=None):
 nf=COUNTS[layout];control=control_offset is not None;blocks=3 if control else 6
 schedule,geometry,widths,eligible=plan(layout,kind);owed=[True]*nf
 def code(ch,band,b):return (7+ch*7+band*3+(5 if kind=='updates'and b>=3 and ch%2==0 else 0))%32
 def fresh(ch,b):return b==0 or(ch==5 and geometry(schedule[b])[0]!=geometry(schedule[b-1])[0])or(ch<5 and widths[b][ch]!=widths[b-1][ch])
 def val(plane,i,b):return conventional(spectrum(i,plane,basis),b)
 w=BitWriter().put(0x0b77,16).put(2 if converted else 0,2).put(0,3).put(2047,11)
 w.put(0,2).put(2 if control else 3,2).put(layout,3).put(int(lfe),1).put(16,5).put(31,5).zeros(3)
 if not converted and control:w.put(0,1)
 if converted:
  if control:w.put(1,1)
  w.put(20,6)
 w.put(0,1)
 if not control:w.put(1,1).put(1,1)
 w.put(0,2).zeros(8)
 w.put(int(not control),1)
 for b in range(1,blocks):
  changed=not control and schedule[b]!=schedule[b-1];w.put(int(changed),1)
  if changed:w.put(1,1)
 for b in range(blocks):
  if not control:w.put(3 if fresh(5,b)else 0,2)
  for ch in range(nf):w.put(3 if (b==0 if control else fresh(ch,b))else 0,2)
 if lfe:w.put(1,1).zeros(blocks-1)
 if not converted:
  if not control:w.zeros(5*nf)
  else:w.put(0,1)
 if not control:
  if 5 in eligible:w.put(int(5 in active),1)
  for ch in range(nf):
   if ch in eligible:w.put(int(ch in active),1)
  if lfe:w.put(int(6 in active),1)
 w.put(63,6).put(15,4).put(0,1)
 for local in range(blocks):
  b=local+(control_offset or 0);state=schedule[b];mode,begin,members=state;start,bands,mapping=geometry(state)
  changed=b>0 and state!=schedule[b-1];w.zeros(2)
  if not control:
   if b==0 or changed:
    w.put(int(mode=='e'),1)
    if layout!=2:
     for ch in range(nf):w.put(int(ch in members),1)
    if mode=='e':w.put(begin,4).put(0,4).put(0,1) # end sub-band 7 / bin 73
    else:
     if layout==2:w.put(0,1)
     w.put((start-37)//12,4).put(0,4).put(0,1)
   if mode=='e':w.put(0,1)
   first=True
   for ch in range(nf):
    if ch not in members:owed[ch]=True;continue
    update=owed[ch]or changed or(kind=='updates'and b==3 and ch%2==0)
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
   if local==0:w.zeros(4 if control else 2)
   else:w.put(0,1)
  for ch in range(nf):
   if (local==0 if control else fresh(ch,b))and(control or ch not in members):w.put(0,6)
  if not control and fresh(5,b):
   w.put(5,4)
   for _ in range((73-start)//12):w.put(62,7)
  for ch in range(nf):
   if (local==0 if control else fresh(ch,b)):
    end=73 if control else widths[b][ch];w.put(10,4)
    for _ in range((end+8)//12):w.put(62,7)
    w.zeros(2)
  if lfe and local==0:w.put(10,4).put(62,7).put(62,7)
  if not converted:w.put(0,1)
  if not control:
   if b==0:w.zeros(6)
   else:w.put(0,1)
  def mantissas(plane,lo,hi):
   if not control and plane in active:
    if b:return
    w.put(0,2)
    for i in range(lo,hi):
     for q in spectrum(i,2 if plane==5 else 1 if plane==6 else plane,basis):w.put(q&65535,16)
   else:
    for i in range(lo,hi):
     value=val(2 if plane==5 else 1 if plane==6 else plane,i,b)
     if control and plane<nf and plane in members and i>=start:value=val(2,i,b)*amplitude(code(plane,mapping[i],b))
     w.put(round(value)&65535,16)
  written=False
  for ch in range(nf):
   mantissas(ch,0,73 if control else widths[b][ch])
   if not control and ch in members and not written:mantissas(5,start,73);written=True
  if lfe:mantissas(6,0,7)
 return patch_crc(w.to_bytes(4096))
def generate():
 files={};cases=[]
 for layout in COUNTS:
  for kind in ['updates','e-o-e','geometry']+(['join']if COUNTS[layout]>2 else[]):
   schedule,_,_,base=plan(layout,kind)
   for lfe in [False,True]:
    eligible=base+([6]if lfe else[])
    if not eligible:continue
    masks=[]
    for active in[eligible,eligible[::2]]+([[6]]if lfe else[]):
     if active not in masks:masks.append(active)
    for converted in[False,True]:
     for active in masks:
      for basis in[3,'mixed']:
       mask=sum(1<<p for p in active);name=f'l{layout}-f{int(lfe)}-t{2 if converted else 0}-{kind}-m{mask}-b{basis}'
       raw=packet(layout,lfe,converted,kind,active,basis)
       # The uncoupled control uses two 3-block frames per 6-block AHT
       # frame, keeping even 5.1 scalar controls within the 4096-byte limit.
       controls=[packet(layout,lfe,converted,kind,[],basis,off)for off in[0,3]]*3
       for suffix,data in[('.ec3',raw*3),('-control.ec3',b''.join(controls)),('.mkv',movie([raw]*3,6,COUNTS[layout]+int(lfe))),('-control.mkv',movie(controls,3,COUNTS[layout]+int(lfe)))]:files[name+suffix]=data
       cases.append(dict(name=name,file=name+'.ec3',control=name+'-control.ec3',video=name+'.mkv',control_video=name+'-control.mkv',blocks=6,channels=COUNTS[layout]+int(lfe),layout=layout,lfe=lfe,kind=kind,active=active,eligible=eligible,basis=basis,quantization_bound=4*73/(32768*1024),exact=False))
 files['manifest.json']=(json.dumps(dict(cases=cases),indent=2)+'\n').encode();return files
if __name__=='__main__':
 OUT.mkdir(parents=True,exist_ok=True)
 for n,b in generate().items():(OUT/n).write_bytes(b)
