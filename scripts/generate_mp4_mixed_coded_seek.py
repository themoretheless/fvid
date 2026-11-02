"""Authored AAC/ALAC/PCM mixtures on an exact rational media/edit clock."""
from pathlib import Path
from fractions import Fraction as F
import json,struct,hashlib
from generate_mp4_audio_reconfiguration import generate as pcm
from generate_mp4_alac_transitions import convert
from generate_mp4_aac_transitions import generate as aac,SCALE
from generate_mp4_variable_rate_edits import edit_movie
from generate_mp4_fragment_description_regressions import atoms
from generate_vp9_chroma import atom,word,frame,mp4
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/mp4-mixed-coded-seek'
def extract(data):
 top=dict(atoms(data));trak=next(b for k,b in atoms(top[b'moov'])if k==b'trak'and b'soun'in b);mdia=dict(atoms(trak))[b'mdia'];tables=dict(atoms(dict(atoms(dict(atoms(mdia))[b'minf']))[b'stbl']))
 entries=[atom(k,b)for k,b in atoms(tables[b'stsd'][8:])];fixed,n=struct.unpack_from('>II',tables[b'stsz'],4);sizes=[fixed]*n if fixed else struct.unpack_from('>'+'I'*n,tables[b'stsz'],12);offsets=struct.unpack_from('>'+'I'*n,tables[b'stco'],8)
 rows=list(struct.iter_unpack('>III',tables[b'stsc'][8:]));packets=[]
 for i,(offset,size)in enumerate(zip(offsets,sizes),1):
  _,count,identity=next(row for row in reversed(rows)if row[0]<=i);assert count==1
  packets.append((identity-1,data[offset:offset+size]))
 return entries,packets

def generate():
 source=pcm(pcm_only=True,pcm_frames=[96,64,80,144,96,112]);plain=source['pcm.mp4'];raw=convert(plain);rice=convert(plain,True);coded=aac();p_entries,p_packets=extract(plain);r_entries,r_packets=extract(raw);z_entries,z_packets=extract(rice)
 rates=[48000,32000,44100];channels=[2,1,2];counts=[96,64,80,144,96,112];catalog=[]
 for i in range(3):catalog.append(dict(entry=p_entries[i],rate=rates[i],channels=channels[i],family='pcm',packets=[(p_packets[i][1],counts[i])]))
 for i in range(3):
  entries,packets=(z_entries,z_packets)if i==2 else(r_entries,r_packets)
  catalog.append(dict(entry=entries[i],rate=rates[i],channels=channels[i],family='alac',packets=[(packets[j][1],counts[j])for j in [i,i+3]]))
 for i in range(3):
  entries,packets=extract(coded[f'control-{i}.mp4']);n=960 if i==1 else 1024
  catalog.append(dict(entry=entries[0],rate=rates[i],channels=channels[i],family='aac',packets=[(packet,n)for _,packet in packets[:2]]))
 video=dict(atoms(mp4([frame(8,'420',16,16)]*6,8,'420',(16,16),list(range(1,7)))))
 files={};records=[]
 def movie(selection,mode):
  order=sorted(set(selection),reverse=mode=='third');packets=[];groups=[];at=0;native=0
  for identity in selection:
   c=catalog[identity];groups.append((identity,F(at,SCALE)))
   for data,n in c['packets']:
    assert n*SCALE%c['rate']==0;d=n*SCALE//c['rate'];packets.append((order.index(identity)+1,data,d));at+=d;native+=n
  total=at;header=atom(b'ftyp',video[b'ftyp']);offset=len(header)+8+len(video[b'mdat']);offsets=[]
  for _,data,_ in packets:offsets.append(offset);offset+=len(data)
  n=len(packets);stbl=atom(b'stbl',atom(b'stsd',word(0,len(order))+b''.join(catalog[i]['entry']for i in order))+atom(b'stts',word(0,n)+b''.join(word(1,d)for _,_,d in packets))+atom(b'stsz',word(0,0,n,*[len(data)for _,data,_ in packets]))+atom(b'stsc',word(0,n)+b''.join(word(i+1,1,identity)for i,(identity,_,_)in enumerate(packets)))+atom(b'stco',word(0,n,*offsets)))
  tkhd=bytearray(84);tkhd[:4]=word(3);tkhd[12:16]=word(2);tkhd[20:24]=word((total*50+SCALE-1)//SCALE);tkhd[40:44]=tkhd[56:60]=word(65536);tkhd[72:76]=word(0x40000000);mdhd=bytearray(24);mdhd[12:20]=word(SCALE,total)
  track=atom(b'trak',atom(b'tkhd',tkhd)+atom(b'mdia',atom(b'mdhd',mdhd)+atom(b'hdlr',word(0,0)+b'soun'+bytes(12))+atom(b'minf',atom(b'dinf',atom(b'dref',word(0,1)+atom(b'url ',word(1))))+stbl)))
  movie_boxes=[]
  for tag,b in atoms(video[b'moov']):
   if tag==b'mvhd':
    b=bytearray(b);old=int.from_bytes(b[16:20],'big');b[16:20]=word(max(old,(total*50+SCALE-1)//SCALE))
   movie_boxes.append(atom(tag,b))
  body=b''.join(movie_boxes)+track
  if mode in ['classic','third']:return header+atom(b'mdat',video[b'mdat']+b''.join(data for _,data,_ in packets))+atom(b'moov',body),total,native,groups
  hybrid=mode=='hybrid';first=packets[0]
  def init(body,audio=False):
   out=b''
   for k,b in atoms(body):
    if k==b'trak':b=init(b,b'soun'in b)
    elif k in [b'mdia',b'minf',b'stbl']:b=init(b,audio)
    elif audio and k==b'stts':b=word(0,1,1,first[2])if hybrid else word(0,0)
    elif audio and k==b'stsz':b=word(0,0,1,len(first[1]))if hybrid else word(0,0,0)
    elif audio and k==b'stsc':b=word(0,1,1,1,first[0])if hybrid else word(0,0)
    elif audio and k==b'stco':b=word(0,1,offsets[0])if hybrid else word(0,0)
    out+=atom(k,b)
   return out
  brand=bytearray(video[b'ftyp']);brand[:4]=b'iso5';brand[8:12]=b'iso5';result=atom(b'ftyp',brand)+atom(b'mdat',video[b'mdat']+(first[1]if hybrid else b''))+atom(b'moov',init(body)+atom(b'mvex',atom(b'trex',word(0,2,first[0],0,0,0x02000000))))
  at=first[2]if hybrid else 0
  for seq,(identity,data,d)in enumerate(packets[1:]if hybrid else packets,1):
   tfhd=atom(b'tfhd',word(0x02003a,2,identity,d,len(data),0x02000000))
   def moof(offset):return atom(b'moof',atom(b'mfhd',word(0,seq))+atom(b'traf',tfhd+atom(b'tfdt',word(0,at))+atom(b'trun',word(1,1,offset))))
   b=moof(0);result+=moof(len(b)+8)+atom(b'mdat',data);at+=d
  assert at==total;return result,total,native,groups
 def add(name,data,total,native,groups,edited):
  duration=F(total,SCALE);end=duration;queries={0,1,500001,1000000,1000001};seen=set()
  for identity,start in groups:
   if identity in seen:continue
   seen.add(identity);probe=max(start,F(1,1000)if edited else F(0))+F(500001,10**9);queries.add(int(probe*10**9))
   if edited:queries.add(int((probe+duration+F(1,1000))*10**9))
  if edited:
   data=edit_movie(data,[(SCALE//1000,-1),(total-SCALE//1000,SCALE//1000),(2*SCALE//1000,-1),(total-SCALE//1000,SCALE//1000)],clock=SCALE)
   end=2*duration+F(1,1000);native=2*native+48;queries.update([int(duration*10**9)+1,int((duration+F(1,1000))*10**9)+1])
  floor=int(end*10**9);queries.update([floor-1,floor,floor+1]);files[name+'.mp4']=data
  records.append(dict(file=name+'.mp4',native_frames=native,end_num=end.numerator,end_den=end.denominator,opening_silence_frames=48 if edited else 0,video_frames=6,queries_ns=sorted(queries)))
 for family,selection,modes in [('alac',[0,4,2,3,1,5,0],['classic','third','fragmented']),('aac',[0,7,2,6,1,8,0],['classic','third','fragmented']),('all',[0,4,6,2,7,3,1,8,5,0],['classic','hybrid'])]:
  for mode in modes:
   data,total,native,groups=movie(selection,mode)
   for edited in [False,True]:add(f'{family}-{mode}'+('-edits'if edited else''),data,total,native,groups,edited)
 phase,_,_,_=movie([2],'classic');files['pcm-phase.mp4']=phase;files['pcm-phase.f32le']=source['pcm-2.f32le']
 files['manifest.json']=(json.dumps(dict(provenance='Repository-authored short arithmetic PCM,ALAC raw/Rice with varying native frame counts and AAC LC1024/Main960 scalar pulse spectra. Exact14112000Hz media/edit clock;48/32/44.1k mono/stereo;classic/reordered/fragmented/hybrid with one-ms opening silence,two-ms intermediate silence and repeated source window. No private media/parameters,FFmpeg,process or network.',cases=records,raw_phase_fixture=dict(file='pcm-phase.mp4',oracle='pcm-phase.f32le',frames=80,rate=44100,channels=2,scale=SCALE),sha256={n:hashlib.sha256(b).hexdigest()for n,b in files.items()}),indent=2)+'\n').encode();return files
if __name__=='__main__':
 OUT.mkdir(parents=True,exist_ok=True)
 for n,b in generate().items():(OUT/n).write_bytes(b)
