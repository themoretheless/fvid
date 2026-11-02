"""Own aligned-clock PCM/AC3/EAC3 seek,preplay,concealment and programme controls."""
from pathlib import Path
from fractions import Fraction as F
import json,hashlib,struct
from mp4_audio_witness import extract,movie
from generate_mp4_audio_reconfiguration import generate as base
from generate_mp4_ac3_sample_clock import generate as ac3
from generate_eac3_mp4_programmes import generate as programmes
from generate_mp4_variable_rate_edits import edit_movie
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/mp4-mixed-dolby-seek'
SCALE=14112000

def generate():
 p=base(pcm_frames=[96,64,80,144,96,112]);a=ac3();e=programmes();entries,packets=extract(p['pcm.mp4']);catalog=[];files={};cases=[]
 def add(entry,packets,rate,ch,label,control=None):catalog.append(dict(entry=entry,packets=packets,rate=rate,channels=ch,label=label,control=control));return len(catalog)-1
 for i,(rate,ch,n)in enumerate([(48000,2,96),(32000,1,64),(44100,2,80)]):add(entries[i],[(packets[i][1],n)],rate,ch,'pcm')
 for name,source,counts in [('ac3','exact.mp4',[1536,1536]),('grouped','grouped-exact.mp4',[1536,3072]),('concealed','bad-first-exact.mp4',[1536,1536])]:
  es,ps=extract(a[source]);add(es[0],[(raw,n)for (_,raw),n in zip(ps,counts)],48000,6,name)
 for name,rate,ch in [('eac3-control-1.mp4',48000,2),('eac3-control-2.mp4',32000,1)]:
  es,ps=extract(p[name]);identity=ps[0][0];add(es[identity],[(raw,1536)for _,raw in ps[:2]],rate,ch,'eac3')
 for source,control,rate,ch in [('placeholder-rate.mp4','dual-44100-p1-control.mp4',44100,3),('stereo-48000.mp4','stereo-48000-p1-control.mp4',48000,2)]:
  es,ps=extract(e[source]);own_es,own_ps=extract(e[control]);own=[dict(entry=own_es[0],packets=[(raw,1536)for _,raw in own_ps[:2]],rate=rate,channels=ch)];cn=f'programme-{rate}-control.mp4';files[cn]=movie(own,[0],SCALE)[0];add(es[0],[(raw,1536)for _,raw in ps[:2]],rate,ch,'programme',cn)
 # Independent reset controls for every actual setup/group;programme controls
 # above remove other programmes and use ID0 with the same authored spectra.
 for i,c in enumerate(catalog):
  if c['control']is None:
   c['control']=f'control-{i}.mp4';files[c['control']]=movie(catalog,[i],SCALE)[0]
 def emit(name,selection,mode,programme,edited=False):
  data,total,native,groups=movie(catalog,selection,SCALE,mode);duration=F(total,SCALE);end=duration;queries={0,1,500001,1000000,1000001};preplay=[];seen=set()
  for g in groups:
   c=catalog[g['identity']];start=F(g['start'],SCALE);probe=max(start,F(1,1000)if edited else F(0))+F(500001,10**9)
   if g['identity']not in seen:
    seen.add(g['identity']);queries.add(int(probe*10**9));
    if c['label']!='pcm':preplay.append(int(probe*10**9))
    if edited:queries.add(int((probe+duration+F(1,1000))*10**9))
  if edited:data=edit_movie(data,[(SCALE//1000,-1),(total-SCALE//1000,SCALE//1000),(2*SCALE//1000,-1),(total-SCALE//1000,SCALE//1000)],clock=SCALE);end=2*duration+F(1,1000);native=2*native+48
  end_ns=int(end*10**9);queries.update([end_ns-1,end_ns,end_ns+1]);files[name+'.mp4']=data;cases.append(dict(file=name+'.mp4',programme=programme,native_frames=native,end_num=end.numerator,end_den=end.denominator,source_num=duration.numerator,source_den=duration.denominator,opening_silence_frames=48 if edited else 0,groups=groups,edited=edited,queries_ns=sorted(queries),preplay_ns=preplay))
 for family,selection,programme,modes in [('ac3',[0,3,1,4,0],None,['classic','fragmented']),('eac3',[0,6,2,7,0],0,['classic','fragmented']),('programme',[0,8,1,9,0],1,['classic','fragmented','third','hybrid'])]:
  for mode in modes:
   for edited in [False,True]:emit(family+'-'+mode+('-edits'if edited else''),selection,mode,programme,edited)
 emit('combined-classic-edits',[0,3,7,1,6,4,0],'classic',0,True)
 emit('combined-hybrid',[0,3,7,1,6,4,0],'hybrid',0)
 emit('concealed-classic-edits',[0,5,0],'classic',None,True)
 es,ps=extract(e['without-programme7.mp4']);missing=add(es[0],[(raw,1536)for _,raw in ps[:2]],48000,3,'missing');files['missing-programme.mp4']=movie(catalog,[0,missing,0],SCALE)[0]
 budget=len(catalog[3]['packets'][0][0]);assert len(catalog[0]['packets'][0][0])<=budget<len(catalog[4]['packets'][1][0]);budget_seek=F(2+64+2+32,1000)+F(500001,10**9)
 files['manifest.json']=(json.dumps(dict(provenance='Own short arithmetic PCM,existing repository-authored synthetic AC3 5.1 tone and authored EAC3 spectra/multiplexes. Exact aligned14112000Hz clock,classic/reordered/fragmented/hybrid,silence/repeated windows,selected programme1 and actual44.1k with48k metadata hint. Concealed CRC-bad AC3 is a successful concealment control;missing programme7 and packet budget are separate refusal controls. No private media/parameters,FFmpeg,process or network.',cases=cases,refusal=dict(file='missing-programme.mp4',programme=7),budget=dict(file='ac3-classic.mp4',bytes=budget,target_ns=int(budget_seek*10**9)),sha256={n:hashlib.sha256(b).hexdigest()for n,b in files.items()}),indent=2)+'\n').encode();return files
if __name__=='__main__':
 OUT.mkdir(parents=True,exist_ok=True)
 for n,b in generate().items():(OUT/n).write_bytes(b)
