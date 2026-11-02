"""Pure authored MP4 audio tables;fixture generation only."""
import struct
from generate_mp4_fragment_description_regressions import atoms
from generate_vp9_chroma import atom,word,frame,mp4

def extract(data):
 top=dict(atoms(data));track=next(b for k,b in atoms(top[b'moov'])if k==b'trak'and b'soun'in b);mdia=dict(atoms(track))[b'mdia'];tables=dict(atoms(dict(atoms(dict(atoms(mdia))[b'minf']))[b'stbl']));entries=[atom(k,b)for k,b in atoms(tables[b'stsd'][8:])]
 fixed,n=struct.unpack_from('>II',tables[b'stsz'],4);sizes=[fixed]*n if fixed else struct.unpack_from('>'+'I'*n,tables[b'stsz'],12);nc=int.from_bytes(tables[b'stco'][4:8],'big');offsets=struct.unpack_from('>'+'I'*nc,tables[b'stco'],8);rows=list(struct.iter_unpack('>III',tables[b'stsc'][8:]));out=[];i=0
 for chunk,start in enumerate(offsets,1):
  _,count,identity=next(row for row in reversed(rows)if row[0]<=chunk);at=start
  for _ in range(count):
   assert i<n;raw=data[at:at+sizes[i]];assert len(raw)==sizes[i];out.append((identity-1,raw));at+=sizes[i];i+=1
 assert i==n;return entries,out

def movie(catalog,selection,scale,mode='classic'):
 video=dict(atoms(mp4([frame(8,'420',16,16)]*6,8,'420',(16,16),list(range(1,7)))));order=sorted(set(selection),reverse=mode=='third');packets=[];groups=[];at=0;native=0
 for identity in selection:
  c=catalog[identity];groups.append(dict(identity=identity,start=at,rate=c['rate'],channels=c['channels'],frames=sum(n for _,n in c['packets']),control=c.get('control')))
  for raw,n in c['packets']:
   assert n*scale%c['rate']==0;d=n*scale//c['rate'];packets.append((order.index(identity)+1,raw,d));at+=d;native+=n
 total=at;header=atom(b'ftyp',video[b'ftyp']);offset=len(header)+8+len(video[b'mdat']);offsets=[]
 for _,raw,_ in packets:offsets.append(offset);offset+=len(raw)
 n=len(packets);stbl=atom(b'stbl',atom(b'stsd',word(0,len(order))+b''.join(catalog[i]['entry']for i in order))+atom(b'stts',word(0,n)+b''.join(word(1,d)for _,_,d in packets))+atom(b'stsz',word(0,0,n,*[len(raw)for _,raw,_ in packets]))+atom(b'stsc',word(0,n)+b''.join(word(i+1,1,identity)for i,(identity,_,_)in enumerate(packets)))+atom(b'stco',word(0,n,*offsets)))
 tkhd=bytearray(84);tkhd[:4]=word(3);tkhd[12:16]=word(2);tkhd[20:24]=word((total*50+scale-1)//scale);tkhd[40:44]=tkhd[56:60]=word(65536);tkhd[72:76]=word(0x40000000);mdhd=bytearray(24);mdhd[12:20]=word(scale,total)
 track=atom(b'trak',atom(b'tkhd',tkhd)+atom(b'mdia',atom(b'mdhd',mdhd)+atom(b'hdlr',word(0,0)+b'soun'+bytes(12))+atom(b'minf',atom(b'dinf',atom(b'dref',word(0,1)+atom(b'url ',word(1))))+stbl)))
 boxes=[]
 for k,b in atoms(video[b'moov']):
  if k==b'mvhd':b=bytearray(b);b[16:20]=word(max(int.from_bytes(b[16:20],'big'),(total*50+scale-1)//scale))
  boxes.append(atom(k,b))
 body=b''.join(boxes)+track
 if mode in ['classic','third']:return header+atom(b'mdat',video[b'mdat']+b''.join(raw for _,raw,_ in packets))+atom(b'moov',body),total,native,groups
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
 for seq,(identity,raw,d)in enumerate(packets[1:]if hybrid else packets,1):
  tfhd=atom(b'tfhd',word(0x02003a,2,identity,d,len(raw),0x02000000))
  def moof(offset):return atom(b'moof',atom(b'mfhd',word(0,seq))+atom(b'traf',tfhd+atom(b'tfdt',word(0,at))+atom(b'trun',word(1,1,offset))))
  b=moof(0);result+=moof(len(b)+8)+atom(b'mdat',raw);at+=d
 assert at==total;return result,total,native,groups
