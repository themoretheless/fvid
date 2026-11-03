"""Authored VP9/tx3g movies with co-timed unspecified subtitle endpoints."""
from pathlib import Path
import json,struct,hashlib
from generate_mp4_seek_io_regressions import generate
from generate_mp4_tfhd_defaults_regressions import atoms
from generate_mp4_subtitle_negative_clock import text_entry
from generate_vp9_chroma import atom,word
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/mp4-subtitle-equal-pts'
def generate_files():
 own=generate();parts=list(atoms(own['three-colors.mp4']));prefix=b''.join(atom(k,b) for k,b in parts[:2])
 def stretch(data):
  out=b''
  for k,b in atoms(data):
   if k in [b'trak',b'mdia',b'minf',b'stbl']:b=stretch(b)
   elif k in [b'mvhd',b'mdhd']:b=b[:16]+word(300)+b[20:]
   elif k==b'tkhd':b=b[:20]+word(300)+b[24:]
   elif k==b'stts':b=word(0,1,3,100)
   out+=atom(k,b)
  return out
 base=stretch(parts[2][1]);track=next(b for k,b in atoms(base)if k==b'trak')
 texts=['first co-timed line','second co-timed line','distinct later line'];packets=[struct.pack('>H',len(t))+t.encode()for t in texts]
 def subtitle(data,offset,scale,pts,durations,identity=False):
  out=b''
  for k,b in atoms(data):
   if k in [b'mdia',b'minf']:b=subtitle(b,offset,scale,pts,durations)
   elif k==b'tkhd':b=b[:12]+word(2)+b[16:20]+word(300)+b[24:]
   elif k==b'mdhd':b=b[:12]+word(scale,sum(durations))+b[20:]
   elif k==b'hdlr':b=word(0,0)+b'sbtl'+bytes(12)+b'Co-timed text\0'
   elif k==b'vmhd':k=b'nmhd';b=word(0)
   elif k==b'stbl':
    dts=[0,durations[0],sum(durations[:2])]
    b=atom(b'stsd',word(0,1)+text_entry())+atom(b'stts',word(0,3)+b''.join(word(1,n)for n in durations))+atom(b'stsz',word(0,0,3,*map(len,packets)))+atom(b'stsc',word(0,1,1,3,1))+atom(b'stco',word(0,1,offset))+atom(b'ctts',word(1<<24,3)+b''.join(struct.pack('>Ii',1,p-d)for p,d in zip(pts,dts)))
   out+=atom(k,b)
  if identity:out+=atom(b'edts',atom(b'elst',word(0,1,200,0,65536)))
  return out
 files={f'frame-{i}.yuv16':own[f'frame-{i}.yuv16']for i in range(3)};cases=[]
 for scale in [50,1001]:
  for mode,pts,durations in [('positive',[0,0,5*scale],[0,0,1]),('negative',[-3,-3,3*scale],[0,0,1]),('explicit-control',[0,0,5*scale],[1,0,1]),('terminal-control',[0,5*scale,5*scale],[1,0,0])]:
   for identity in [False,True]:
    name=f'{mode}-{scale}-{"identity"if identity else "plain"}.mp4'
    size=len(atom(b'moov',base+atom(b'trak',subtitle(track,0,scale,pts,durations,identity))));sub=subtitle(track,len(prefix)+size+8,scale,pts,durations,identity)
    files[name]=prefix+atom(b'moov',base+atom(b'trak',sub))+atom(b'mdat',b''.join(packets))
    cues=[]
    for i,p in enumerate(pts):
     later=sorted(set(t for t in pts if t>p));end=p+durations[i]if durations[i]else later[0]if later else p+2*scale
     start_ns=max(p,0)*1_000_000_000//scale;end_ns=max(end,0)*1_000_000_000//scale
     if identity:start_ns=min(start_ns,4_000_000_000);end_ns=min(end_ns,4_000_000_000)
     if end_ns>start_ns:cues.append(dict(text=texts[i],start_ns=start_ns,end_ns=end_ns))
    control_durations=[]
    for i,p in enumerate(pts):
     later=sorted(set(t for t in pts if t>p));control_durations.append(durations[i]or(later[0]-p if later else 2*scale))
    control=name.removesuffix('.mp4')+'-explicit-spans.mp4'
    size=len(atom(b'moov',base+atom(b'trak',subtitle(track,0,scale,pts,control_durations,identity))));sub=subtitle(track,len(prefix)+size+8,scale,pts,control_durations,identity)
    files[control]=prefix+atom(b'moov',base+atom(b'trak',sub))+atom(b'mdat',b''.join(packets))
    cases.append(dict(file=name,control=control,control_durations=control_durations,scale=scale,pts=pts,durations=durations,texts=texts,identity=identity,mode=mode,cues=cues))
 files['manifest.json']=(json.dumps(dict(provenance='Authored VP9 pixels and invented tx3g strings; zero/explicit STTS durations, signed CTTS, co-timed groups, no encoder, network or private media.',cases=cases,sha256={n:hashlib.sha256(v).hexdigest()for n,v in files.items()}),indent=2)+'\n').encode();return files
if __name__=='__main__':
 OUT.mkdir(parents=True,exist_ok=True)
 for n,v in generate_files().items():(OUT/n).write_bytes(v)
