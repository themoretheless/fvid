"""Authored PCM/MACE chunk-budget controls behind unused coded descriptions."""
from pathlib import Path
import json,struct
from generate_mp4_enabled_track_regressions import boxes,canonical_movie,rewrite_track
from generate_mp4_audio_clock_regressions import generate as audio_clocks
from generate_vp9_chroma import mp4,frame
from generate_audio_resample_window_fixture import atom,word
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/mp4-unused-index-geometry'
def generate():
 base=dict(boxes(mp4([frame(8,'420',16,16)]*3,8,'420',(16,16),sync_samples=[1,2,3])));ftyp=atom(b'ftyp',base[b'ftyp']);movie=dict(boxes(base[b'moov']));files={};cases=[];controls=[]
 def find_entry(data,codec):
  for k,b in boxes(data):
   if k==b'stsd':
    for tag,body in boxes(b[8:]):
     if tag==codec:return atom(tag,body)
   elif k in [b'moov',b'trak',b'mdia',b'minf',b'stbl']:
    result=find_entry(b,codec)
    if result:return result
 def header(codec,channels=1,bits=8):
  e=bytearray(28);e[6:8]=b'\0\1';e[16:20]=struct.pack('>HH',channels,bits);e[24:28]=word(48000<<16);return atom(codec,e)
 unused={b'alac':find_entry(audio_clocks()['alac-48000.mp4'],b'alac'),b'ima4':header(b'ima4',2,4),b'MAC3':header(b'MAC3',2),b'MAC6':header(b'MAC6',2),b'fl32':header(b'fl32',2,32)+b''}
 def make(entries,selected,count,size,payload):
  stbl=atom(b'stbl',atom(b'stsd',word(0,len(entries))+b''.join(entries))+atom(b'stts',word(0,1,count,1))+atom(b'stsc',word(0,1,1,count,selected))+atom(b'stsz',word(0,size,count))+atom(b'stco',word(0,1,len(ftyp)+8+len(base[b'mdat']))))
  dinf=atom(b'dinf',atom(b'dref',word(0,1)+atom(b'url ',word(1))));mdhd=bytearray(24);mdhd[12:20]=word(48000,count);tkhd=bytearray(84);tkhd[:4]=word(3);tkhd[12:16]=word(2);tkhd[20:24]=word(3);tkhd[40:44]=tkhd[56:60]=word(65536);tkhd[72:76]=word(1<<30)
  audio=atom(b'trak',atom(b'tkhd',tkhd)+atom(b'mdia',atom(b'mdhd',mdhd)+atom(b'hdlr',word(0,0)+b'soun'+bytes(12))+atom(b'minf',dinf+stbl)))
  video=atom(b'trak',rewrite_track(movie[b'trak'],1,3,len(ftyp)+8,False))
  return ftyp+atom(b'mdat',base[b'mdat']+payload)+atom(b'moov',atom(b'mvhd',canonical_movie(movie[b'mvhd'],3))+video+audio)
 for codec,count,size,payload,unit in [(b'sowt',64,2,struct.pack('<h',4096)*64,1),(b'sowt',2048,2,struct.pack('<h',4096)*2048,1),(b'MAC3',72,1,bytes((i*29+7)%256 for i in range(24)),6),(b'MAC6',72,1,bytes((i*29+7)%256 for i in range(12)),6)]:
  stem=codec.decode()+f'-{count}';actual=header(codec,1,16 if codec==b'sowt'else 8);control=stem+'-control.mp4';files[control]=make([actual],1,count,size,payload);controls.append(dict(file=control,frames=count,units=count//unit,codec=codec.decode()))
  for tag,entry in unused.items():
   for first in [False,True]:
    name=stem+'-unused-'+tag.decode()+('-first'if first else'-last')+'.mp4';files[name]=make([entry,actual]if first else[actual,entry],2 if first else 1,count,size,payload);cases.append(dict(file=name,control=control,frames=count,units=count//unit,codec=codec.decode(),selected=2 if first else 1))
 bad=[]
 pcm=struct.pack('<h',4096)*64
 for name,selected in [('zero-description',0),('large-description',3)]:
  file='bad-'+name+'.mp4';files[file]=make([unused[b'alac'],header(b'sowt',1,16)],selected,64,2,pcm);bad.append(dict(file=file,needle='invalid sample-to-chunk mapping'))
 file='bad-mace-partial-block.mp4';files[file]=make([header(b'MAC3')],1,73,1,bytes((i*29+7)%256 for i in range(24)));bad.append(dict(file=file,needle='sample count is not a whole number of blocks'))
 opaque=bytearray(header(b'MAC3'));opaque[16:18]=struct.pack('>H',2)
 file='bad-selected-opaque-mace.mp4';files[file]=make([bytes(opaque),header(b'MAC3')],1,72,1,bytes((i*29+7)%256 for i in range(24)));bad.append(dict(file=file,needle='zero MP4 sample size'))
 complex_cases=[]
 def shifted(data,audio=False):
  out=b''
  for tag,body in boxes(data):
   if tag==b'trak':body=shifted(body,b'soun'in body)
   elif tag in [b'moov',b'mdia',b'minf',b'stbl']:
    body=shifted(body,audio)
    if tag==b'stbl'and audio:
     sizes=dict(boxes(body))[b'stsz'];count=int.from_bytes(sizes[8:12],'big');body+=atom(b'ctts',word(0,1,count,6))
   out+=atom(tag,body)
  return out
 for stem,units in [('sowt-64',64),('MAC3-72',12)]:
  control=stem+'-ctts-control.mp4';files[control]=shifted(files[stem+'-control.mp4'])
  file=stem+'-ctts-unused-first.mp4';files[file]=shifted(files[stem+'-unused-'+('MAC3'if stem.startswith('sowt')else'alac')+'-first.mp4']);complex_cases.append(dict(file=file,control=control,units=units))
 files['manifest.json']=(json.dumps(dict(provenance='Authored mono PCM constants and arithmetic MACE codewords beside three neutral VP9 pictures; one audio chunk and explicit whole-run stsc selection. Dormant ALAC,IMA4,MACE,float descriptions. No encoder,private media or network.',controls=controls,cases=cases,bad=bad,complex=complex_cases),indent=2)+'\n').encode();return files
if __name__=='__main__':
 OUT.mkdir(parents=True,exist_ok=True)
 for n,b in generate().items():(OUT/n).write_bytes(b)
