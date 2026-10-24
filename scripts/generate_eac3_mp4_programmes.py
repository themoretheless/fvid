"""Owned compact E-AC-3 multiplexes in MP4; no encoder, process or network."""
import json,struct
from pathlib import Path
from generate_eac3_dependent import packet,COUNTS
from generate_eac3_programme_selection import identify
from generate_eac3_dialogue_normalization import level
from generate_eac3_aht_lfe import audiovisual
from generate_eac3_mp4_presentation import rewrite
from generate_eac3_playback import BitWriter
from generate_mp4_enabled_track_regressions import boxes
from generate_audio_resample_window_fixture import atom
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/eac3-mp4-programmes'
RATES=[48000,44100,32000]
def config(records,groups,rate):
 peak=(max(map(len,groups))*8*rate+1535999)//1536000
 assert 0<peak<=8191,peak
 w=BitWriter().put(peak,13).put(len(records)-1,3)
 for layout,lfe,r,dep in records:
  w.put(r,2).put(16,5).zeros(5).put(layout,3).put(int(lfe),1).zeros(3).put(dep,4).put(2 if dep else 0,9 if dep else 1)
 assert len(w.bits)%8==0
 return bytes(sum(w.bits[i+j]<<(7-j)for j in range(8))for i in range(0,len(w.bits),8))
def movie(groups,records,rate):return rewrite(audiovisual(groups),config(records,groups,rate),rate)
def transform(data,*,edits=False,clock=False,multiple=False,placeholder=False,audio=False,sizes=None):
 out=b''
 for kind,body in boxes(data):
  if kind in [b'moov',b'trak',b'mdia',b'minf',b'stbl']:
   selected=(b'ec-3'in body)if kind==b'trak'else audio
   if kind==b'stbl' and selected and multiple:
    stsz=dict(boxes(body))[b'stsz'];fixed,count=struct.unpack_from('>II',stsz,4)
    sizes=[fixed]*count if fixed else list(struct.unpack_from('>'+str(count)+'I',stsz,12))
   body=transform(body,sizes=sizes,edits=edits,clock=clock,multiple=multiple,placeholder=placeholder,audio=selected)
   if kind==b'trak'and selected and edits:
    entries=[(20,-1),(32,256),(32,256)]
    body+=atom(b'edts',atom(b'elst',bytes(4)+struct.pack('>I',3)+b''.join(struct.pack('>IiI',d,s,65536)for d,s in entries)))
  elif audio and kind==b'tkhd'and edits:body=bytearray(body);body[20:24]=struct.pack('>I',84)
  elif audio and kind==b'mdhd'and clock:body=bytearray(body);body[12:20]=struct.pack('>II',1000,96)
  elif audio and kind==b'stts'and clock:body=bytes(4)+struct.pack('>III',1,3,32)
  elif audio and kind==b'stsd':
   if placeholder:
    body=bytearray(body);body[40:44]=struct.pack('>I',48000<<16)
   if multiple:
    assert struct.unpack_from('>I',body,4)[0]==1
    body=body[:4]+struct.pack('>I',2)+body[8:]+body[8:]
  elif audio and kind==b'stco'and multiple:
   assert sizes and struct.unpack_from('>I',body,4)[0]==1
   offset=struct.unpack_from('>I',body,8)[0];offsets=[]
   for size in sizes:offsets.append(offset);offset+=size
   body=bytes(4)+struct.pack('>I',len(offsets))+b''.join(struct.pack('>I',n)for n in offsets)
  elif audio and kind==b'stsc'and multiple:body=bytes(4)+struct.pack('>I',2)+struct.pack('>III',1,1,1)+struct.pack('>III',2,1,2)
  out+=atom(kind,body)
 return out

def generate():
 files={};cases=[];encoded={}
 def add(name,rate,blocks,shapes,expanded=False,dialogue=False):
  r=RATES.index(rate);programmes=[];controls=[];records=[]
  for i,(layout,lfe)in enumerate(shapes):
   groups=[];own=[]
   for f in range(3):
    parts=[];refs=[]
    for cycle in range(6//blocks):
     raw,_=packet(layout,lfe,0,0,'mixed',None,False,blocks_override=blocks,rate_code=r,divisor=1<<i,allocated=not expanded,gains=[64]+[None]*(blocks-1),gains_second=[192]+[None]*(blocks-1)if layout==0 else None,compact=True)
     if expanded:
      dep_layout=2 if f==1 else 0
      dep,_=packet(dep_layout,False,0,0,'mixed',0x0200,True,blocks_override=blocks,rate_code=r,divisor=1<<(i+1),gains=[64]+[None]*(blocks-1),gains_second=[192]+[None]*(blocks-1)if dep_layout==0 else None,compression=16+i,compression_second=240+i if dep_layout==0 else None,transient_windows=[(0,255),(240,127)],compact=True)
     else:dep=b''
     if dialogue:
      raw=level(raw,31-i*3,30-i)
      if dep:dep=level(dep,31-i*3,30-i)
     parts.append(identify(raw,i)+dep);refs.append(raw+dep)
    groups.append(parts);own.append(b''.join(refs))
   record=(layout,lfe,r,int(expanded));records.append(record)
   key=f'{name}-p{i}-control';files[key+'.mp4']=movie(own,[record],rate);controls.append(dict(name=key,channels=8 if expanded else COUNTS[layout]+int(lfe)))
   programmes.append(groups)
  groups=[b''.join(programmes[i][f][cycle]for cycle in range(6//blocks)for i in range(8))for f in range(3)]
  encoded[name]=(groups,records,rate)
  files[name+'.mp4']=movie(groups,records,rate);cases.append(dict(name=name,rate=rate,controls=controls))
 for rate in RATES:
  add(f'dual-{rate}',rate,6,[(0,True)]*8)
  add(f'stereo-{rate}',rate,6,[(2,False)]*8)
  add(f'heterogeneous-{rate}',rate,3,[(i,i%2==1)for i in range(8)])
  for blocks in [3,6]:add(f'expanded-{rate}-b{blocks}',rate,blocks,[(7,True)]*8,True)
 add('51-32000',32000,3,[(7,True)]*8)
 add('expanded-policy-44100-b6',44100,6,[(7,True)]*8,True,True)
 for base,option in [('stereo-48000','edits'),('dual-48000','clock')]:
  c=next(c for c in cases if c['name']==base);name=base+'-'+option;controls=[]
  files[name+'.mp4']=transform(files[base+'.mp4'],**{option:True})
  for control in c['controls']:
   key=control['name']+'-'+option;files[key+'.mp4']=transform(files[control['name']+'.mp4'],**{option:True});controls.append(dict(name=key,channels=control['channels'],plain=control['name']))
  cases.append(dict(name=name,rate=c['rate'],controls=controls,edits=option=='edits'))
 c=next(c for c in cases if c['name']=='dual-44100');files['placeholder-rate.mp4']=transform(files[c['name']+'.mp4'],placeholder=True);cases.append(dict(name='placeholder-rate',rate=44100,controls=c['controls']))
 groups,records,rate=encoded['dual-48000']
 for selected in [1,7]:
  damaged=[]
  for group in groups:
   data=bytearray(group);at=0;current=0
   while at<len(data):
    kind=data[at+2]>>6;identity=(data[at+2]>>3)&7;size=((data[at+2]&7)*256+data[at+3]+1)*2
    if kind!=1:current=identity
    if current!=selected:data[at+6:at+size]=bytes([0xa5])*(size-6)
    at+=size
   damaged.append(bytes(data))
  files[f'corrupt-unselected-p{selected}.mp4']=movie(damaged,records,rate)
 kept=[]
 for group in groups:
  parts=[];at=0
  while at<len(group):
   identity=(group[at+2]>>3)&7;size=((group[at+2]&7)*256+group[at+3]+1)*2
   if identity!=7:parts.append(group[at:at+size])
   at+=size
  kept.append(b''.join(parts))
 files['without-programme7.mp4']=movie(kept,records[:7],rate)
 # Physical track 2 reuses protected media bytes but has its own track ID.
 original=files['heterogeneous-48000.mp4'];result=b''
 for kind,body in boxes(original):
  if kind==b'moov':
   children=list(boxes(body));audio=next(b for k,b in children if k==b'trak'and b'ec-3'in b);clone=b''
   for k,b in boxes(audio):
    if k==b'tkhd':b=bytearray(b);b[12:16]=struct.pack('>I',3)
    clone+=atom(k,b)
   rebuilt=b''
   for k,b in children:
    if k==b'mvhd':b=bytearray(b);b[96:100]=struct.pack('>I',4)
    rebuilt+=atom(k,b)
   body=rebuilt+atom(b'trak',clone)
  result+=atom(kind,body)
 files['two-physical-tracks.mp4']=result
 files['multiple-descriptions.mp4']=transform(files['stereo-48000.mp4'],multiple=True)
 files['manifest.json']=(json.dumps(dict(accepted=True,cases=cases,multiple_control='stereo-48000',multiple_descriptions_accepted=True),indent=2)+'\n').encode();return files
if __name__=='__main__':
 OUT.mkdir(parents=True,exist_ok=True)
 for n,b in generate().items():(OUT/n).write_bytes(b)
