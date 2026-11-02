"""Authored good/bad-CRC/good Dolby seek controls;generation only."""
from pathlib import Path
from fractions import Fraction
import json,hashlib
from mp4_audio_witness import extract,movie
from generate_mp4_audio_reconfiguration import generate as base
from generate_eac3_mp4_programmes import generate as programmes
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/mp4-dolby-concealed-seek'
SCALE=14112000

def damage_selected(raw,programme):
 data=bytearray(raw);at=0;found=False
 while at<len(data):
  size=((data[at+2]&7)*256+data[at+3]+1)*2;kind=data[at+2]>>6;identity=(data[at+2]>>3)&7
  assert size>=8 and at+size<=len(data)
  if kind!=1 and identity==programme:data[at+size-1]^=1;found=True;break
  at+=size
 assert found;return bytes(data)

def generate():
 p=base(pcm_frames=[96,64,80,144,96,112]);e=programmes();entries,packets=extract(p['pcm.mp4']);pcm=dict(entry=entries[0],packets=[(packets[0][1],96)],rate=48000,channels=2);files={};cases=[]
 for label,body,reference,rate,ch,programme in [('stereo',p['eac3-control-1.mp4'],p['eac3-control-1.mp4'],48000,2,0),('programme',e['placeholder-rate.mp4'],e['dual-44100-p1-control.mp4'],44100,3,1)]:
  es,ps=extract(body);entry=es[ps[0][0]];coded=[(ps[0][1],1536),(damage_selected(ps[0][1],programme),1536),(ps[1][1],1536)];catalog=[pcm,dict(entry=entry,packets=coded,rate=rate,channels=ch)]
  rs,rp=extract(reference);control=dict(entry=rs[rp[0][0]],packets=[(rp[0][1],1536),(damage_selected(rp[0][1],0),1536),(rp[1][1],1536)],rate=rate,channels=ch);name=label+'-control.mp4';files[name]=movie([control],[0],SCALE)[0]
  for mode in ['classic','fragmented','hybrid']:
   name2=label+'-'+mode+'.mp4';data,total,native,_=movie(catalog,[0,1,0],SCALE,mode);files[name2]=data;target=Fraction(96,48000)+Fraction(1536,rate)+Fraction(500001,10**9)
   cases.append(dict(file=name2,control=name,programme=programme,rate=rate,channels=ch,frames=native,end_num=total,end_den=SCALE,target_ns=int(target*10**9)))
 files['manifest.json']=(json.dumps(dict(provenance='Authored PCM,neutral VP9 and EAC3 spectra only. A single CRC trailer bit in the selected second independent frame is deliberately invalid;good first/third frames remain unchanged. Successful concealment and seek acceptance,not a missing-programme refusal. No private media,FFmpeg,network or mutation audit.',cases=cases,sha256={n:hashlib.sha256(b).hexdigest()for n,b in files.items()}),indent=2)+'\n').encode();return files
if __name__=='__main__':
 OUT.mkdir(parents=True,exist_ok=True)
 for n,b in generate().items():(OUT/n).write_bytes(b)
