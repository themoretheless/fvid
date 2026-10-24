"""Owned AC-3 RF fields and waveform witnesses; no encoder or network."""
import json
from functools import lru_cache
from pathlib import Path
from generate_eac3_playback import BitWriter,crc16_eac3
from generate_eac3_aht import spectrum,conventional
from generate_eac3_bsid import movie
ROOT=Path(__file__).resolve().parents[1]
OUT=ROOT/'tests/fixtures/playback-errors/ac3-rf-compression'
COUNTS=[2,1,2,3,3,4,4,5]
SIZE=[2560,2786,3840];RATE=[48000,44100,32000]
@lru_cache(None)
def inverse_prefix(length):
 basis={}
 for bit in range(16):
  value=crc16_eac3((1<<bit).to_bytes(2,'big')+bytes(length));mask=1<<bit
  while value:
   top=value.bit_length()-1
   if top not in basis:basis[top]=(value,mask);break
   v,m=basis[top];value^=v;mask^=m
 assert len(basis)==16
 return basis

def crc_frame(raw):
 data=bytearray(raw);split=(len(data)//2*5//8)*2;data[2:4]=b'\0\0';data[-2:]=b'\0\0'
 residue=crc16_eac3(data[4:split]);mask=0;basis=inverse_prefix(split-4)
 while residue:
  v,m=basis[residue.bit_length()-1];residue^=v;mask^=m
 data[2:4]=mask.to_bytes(2,'big');data[-2:]=crc16_eac3(data[split:-2]).to_bytes(2,'big')
 assert crc16_eac3(data[2:split])==crc16_eac3(data[split:])==crc16_eac3(data[2:])==0
 return bytes(data)

def authored(layout,lfe,rate_code,word,word2,*,control=False,clip=False):
 nf=COUNTS[layout];assert nf<=2
 w=BitWriter().put(0x0b77,16).zeros(16).put(rate_code,2).put(36,6).put(8,5).zeros(3).put(layout,3)
 if layout==2:w.zeros(2)
 w.put(int(lfe),1).put(31,5).put(int(word is not None),1)
 if word is not None:w.put(word,8)
 w.zeros(2) # language/audprod
 if layout==0:
  w.put(31,5).put(int(word2 is not None),1)
  if word2 is not None:w.put(word2,8)
  w.zeros(2)
 w.zeros(5) # copyright/original/timecodes/addbsi
 for b in range(6):
  w.zeros(nf*2) # long windows, dither off
  for channel in range(2 if layout==0 else 1):
   code=None if control else ([64,None,224,None,32,None]if channel==0 else[192,32,None,64,None,224])[b]
   w.put(int(code is not None),1)
   if code is not None:w.put(code,8)
  w.put(int(b==0),1)
  if b==0:w.put(0,1) # coupling off
  if layout==2:
   w.put(int(b==0),1)
   if b==0:w.zeros(4)
  for _ in range(nf):w.put(3 if b==0 else 0,2)
  if lfe:w.put(int(b==0),1)
  if b==0:
   w.zeros(6*nf)
   for _ in range(nf):
    w.put(0 if clip and not control else 10,4)
    for j in range(6):w.put(62,7)
    w.zeros(2)
   if lfe:w.put(0 if clip and not control else 10,4).put(62,7).put(62,7)
  w.put(int(b==0),1)
  if b==0:w.put(3,2).put(3,2).put(3,2).put(3,2).put(7,3)
  w.put(int(b==0),1)
  if b==0:
   w.put(63,6)
   for _ in range(nf+int(lfe)):w.put(15,4).put(4,3)
  w.zeros(2) # delta allocation, skip
  for ch in range(nf+int(lfe)):
   for i in range(7 if ch==nf else 73):
    values=[round(v/(1 if clip else 16))for v in spectrum(i,ch,'mixed')]
    w.put(round(conventional(values,b))&65535,16)
 return crc_frame(w.to_bytes(SIZE[rate_code]))

def repack(source,word):
 bits=''.join(f'{v:08b}'for v in source);rate=int(bits[32:34],2);layout=int(bits[48:51],2);assert layout!=0
 at=51+2*int(layout&1!=0 and layout!=1)+2*int(layout&4!=0)+2*int(layout==2)+1+5
 start=at;old=int(bits[at]);at+=1+8*old
 modified=bits[:start]+str(int(word is not None))+(f'{word:08b}'if word is not None else'')+bits[at:-16]
 modified=modified[:34]+f'{36:06b}'+modified[40:]
 assert len(modified)<=(SIZE[rate]-2)*8
 modified=modified.ljust(SIZE[rate]*8,'0')
 return crc_frame(bytes(int(modified[i:i+8],2)for i in range(0,len(modified),8)))

def mp4_movie(packets,layout,lfe,rate):
 from generate_eac3_aht_lfe import audiovisual
 from generate_eac3_mp4_presentation import rewrite
 from generate_mp4_enabled_track_regressions import boxes
 from generate_audio_resample_window_fixture import atom
 config=int(f'{rate:02b}{8:05b}{0:03b}{layout:03b}{int(lfe):01b}{18:05b}'+'00000',2).to_bytes(3,'big')
 data=rewrite(audiovisual(packets),config,RATE[rate])
 def convert(data):
  out=b''
  for kind,body in boxes(data):
   if kind in [b'moov',b'trak',b'mdia',b'minf',b'stbl']:body=convert(body)
   elif kind==b'stsd' and body[12:16]==b'ec-3':
    entry=body[16:44]
    body=body[:8]+atom(b'ac-3',entry+atom(b'dac3',config))
   out+=atom(kind,body)
  return out
 return convert(data)

def generate():
 files={};cases=[]
 def add(label,layout,lfe,rate,code,source=None,clip=False):
  width=COUNTS[layout]+int(lfe);name=f'{label}-c{code}'+('-clip'if clip else'');control_name=label+('-clip'if clip else'')+'-control.mkv'
  if source is None:
   controls=[authored(layout,lfe,rate,None,None,control=True,clip=clip)]*3
   words=[code,None,code^128];words2=[code^255,16,None]if layout==0 else[None]*3
   packets=[authored(layout,lfe,rate,a,b,clip=clip)for a,b in zip(words,words2)];line=[[4,4,.5,.5,2,2],[.25,2,2,4,4,.5]]
  else:
   controls=[repack(p,None)for p in source];words=[code,None,code^128];words2=[None]*3;packets=[repack(p,w)for p,w in zip(source,words)];line=[[1]*6,[1]*6]
  if control_name not in files:files[control_name]=movie(controls,blocks=6,channels=width,rate=RATE[rate],codec='A_AC3')
  files[name+'.ac3']=b''.join(packets);files[name+'.mkv']=movie(packets,blocks=6,channels=width,rate=RATE[rate],codec='A_AC3')
  cases.append(dict(name=name,layout=layout,lfe=lfe,channels=width,rate=RATE[rate],frame_bytes=SIZE[rate],control=control_name,words=words,words2=words2,line=line,clip=clip,scale=1024 if clip else 1))
  if source is not None:
   files[label+'-original.ac3']=b''.join(source);cases[-1]['source']=label+'-original.ac3'
  if code in [0,15,16,127,128,143,240,255]:
   files[name+'.mp4']=mp4_movie(packets,layout,lfe,rate);cases[-1]['mp4']=name+'.mp4'

 for layout in [0,1,2]:
  for lfe in [False,True]:
   for r in range(3):
    for code in (range(256)if r==0 and layout in [0,2]and not lfe else[0,15,16,127,128,143,240,255]):add(f'l{layout}-{int(lfe)}-r{r}',layout,lfe,r,code)
 for layout in [0,1,2]:add(f'l{layout}-1-r0',layout,True,0,127,clip=True)
 for label,source_name,layout,lfe,size in [('coupled','ac3-coupled.ac3',2,False,1024),('51','ac3-51.ac3',7,True,1792)]:
  raw=(ROOT/'tests/fixtures/audio'/source_name).read_bytes();source=[raw[i*size:(i+1)*size]for i in range(3)]
  for p in source:
   split=(len(p)//2*5//8)*2;assert crc16_eac3(p[2:split])==crc16_eac3(p[split:])==0
  for code in [0,15,16,127,128,143,240,255]:add(label,layout,lfe,0,code,source)
 files['manifest.json']=(json.dumps(dict(accepted=True,provenance="Owned AC-3 D45 spectra and repository-authored coupled/5.1 synthetic tones; both CRC regions repaired; no private media, encoder or network.",cases=cases),indent=2)+'\n').encode();return files
if __name__=='__main__':
 OUT.mkdir(parents=True,exist_ok=True)
 for n,b in generate().items():(OUT/n).write_bytes(b)
