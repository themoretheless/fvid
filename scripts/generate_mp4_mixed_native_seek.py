"""Authored mixed PCM/MACE/ADPCM native seek and edit witnesses."""
from pathlib import Path
import json,struct,hashlib
from generate_mp4_mace_transitions import generate as mace
from generate_mp4_adpcm_transitions import generate as adpcm
from generate_mp4_variable_rate_edits import edit_movie
from generate_mp4_fragment_description_regressions import atoms
from generate_vp9_chroma import atom,word,frame,mp4
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/mp4-mixed-native-seek'
def generate():
 files={};records=[];m=mace();a=adpcm()
 def add(name,data,frames,duration,opening_silence):
  files[name+'.mp4']=data;records.append(dict(file=name+'.mp4',frames=frames,duration_ns=duration,video_frames=6,opening_silence_frames=opening_silence))
 for mode,source in [('classic','mixed'),('third','mixed-third'),('fragmented','mixed-fragmented'),('hybrid','mixed-hybrid')]:
  data=m[source+'.mp4'];add('mace-'+mode,data,1200,135000000,0)
  add('mace-'+mode+'-edits',edit_movie(data,[(5,-1),(65,480),(5,-1),(65,480)]),1360,140000000,40)
 entries=[];groups=[]
 for identity in range(3):
  data=a[f'control-{identity}.mp4'];top=dict(atoms(data));trak=next(b for k,b in atoms(top[b'moov'])if k==b'trak' and b'soun'in b)
  mdia=dict(atoms(trak))[b'mdia'];minf=dict(atoms(mdia))[b'minf'];stbl=dict(atoms(dict(atoms(minf))[b'stbl']))
  entries.append(next(atom(k,b)for k,b in atoms(stbl[b'stsd'][8:])))
  fixed,count=struct.unpack_from('>II',stbl[b'stsz'],4);sizes=[fixed]*count if fixed else list(struct.unpack_from('>'+'I'*count,stbl[b'stsz'],12))
  offsets=struct.unpack_from('>'+'I'*count,stbl[b'stco'],8);durations=[]
  for n,d in struct.iter_unpack('>II',stbl[b'stts'][8:]):durations += [d]*n
  assert len(durations)==count==3
  groups.append([(data[o:o+size],d)for o,size,d in zip(offsets,sizes,durations)])
 e=bytearray(28);e[6:8]=struct.pack('>H',1);e[16:20]=struct.pack('>HH',1,16);e[24:28]=word(8000<<16)
 entries.append(atom(b'sowt',e));groups.append([(b''.join(struct.pack('<h',(j*131)%8000-4000)for j in range(64)),16)])
 packets=[(identity,raw,d)for identity in [0,3,1,2,0]for raw,d in groups[identity]];total=sum(d for _,_,d in packets);assert total==216
 video=dict(atoms(mp4([frame(8,'420',16,16)]*6,8,'420',(16,16),list(range(1,7)))))
 def classic(reverse=False):
  order=[3,2,1,0]if reverse else[0,1,2,3];header=atom(b'ftyp',video[b'ftyp']);offset=len(header)+8+len(video[b'mdat']);offsets=[]
  for _,raw,_ in packets:offsets.append(offset);offset+=len(raw)
  n=len(packets);stbl=atom(b'stbl',atom(b'stsd',word(0,len(order))+b''.join(entries[i]for i in order))+atom(b'stts',word(0,n)+b''.join(word(1,d)for _,_,d in packets))+atom(b'stsz',word(0,0,n,*[len(raw)for _,raw,_ in packets]))+atom(b'stsc',word(0,n)+b''.join(word(i+1,1,order.index(identity)+1)for i,(identity,_,_)in enumerate(packets)))+atom(b'stco',word(0,n,*offsets)))
  tkhd=bytearray(84);tkhd[:4]=word(3);tkhd[12:16]=word(2);tkhd[20:24]=word(6);tkhd[40:44]=tkhd[56:60]=word(65536);tkhd[72:76]=word(0x40000000)
  mdhd=bytearray(24);mdhd[12:20]=word(2000,total)
  track=atom(b'trak',atom(b'tkhd',tkhd)+atom(b'mdia',atom(b'mdhd',mdhd)+atom(b'hdlr',word(0,0)+b'soun'+bytes(12))+atom(b'minf',atom(b'dinf',atom(b'dref',word(0,1)+atom(b'url ',word(1))))+stbl)))
  return header+atom(b'mdat',video[b'mdat']+b''.join(raw for _,raw,_ in packets))+atom(b'moov',video[b'moov']+track)
 def fragmented(hybrid=False):
  top=dict(atoms(classic()));first=packets[0]
  def init(body,audio=False):
   out=b''
   for k,b in atoms(body):
    if k==b'trak':b=init(b,b'soun'in b)
    elif k in [b'mdia',b'minf',b'stbl']:b=init(b,audio)
    elif audio and k==b'stts':b=word(0,1,1,first[2])if hybrid else word(0,0)
    elif audio and k==b'stsz':b=word(0,0,1,len(first[1]))if hybrid else word(0,0,0)
    elif audio and k==b'stsc':b=word(0,1,1,1,1)if hybrid else word(0,0)
    elif audio and k==b'stco':b=word(0,1,int.from_bytes(b[8:12],'big'))if hybrid else word(0,0)
    out+=atom(k,b)
   return out
  brand=bytearray(video[b'ftyp']);brand[:4]=b'iso5';brand[8:12]=b'iso5'
  result=atom(b'ftyp',brand)+atom(b'mdat',video[b'mdat']+(first[1]if hybrid else b''))+atom(b'moov',init(top[b'moov'])+atom(b'mvex',atom(b'trex',word(0,2,1,0,0,0x02000000))))
  dts=first[2]if hybrid else 0
  for seq,(identity,raw,duration)in enumerate(packets[1:]if hybrid else packets,1):
   tfhd=atom(b'tfhd',word(0x02003a,2,identity+1,duration,len(raw),0x02000000))
   def moof(offset):return atom(b'moof',atom(b'mfhd',word(0,seq))+atom(b'traf',tfhd+atom(b'tfdt',word(0,dts))+atom(b'trun',word(1,1,offset))))
   box=moof(0);result+=moof(len(box)+8)+atom(b'mdat',raw);dts+=duration
  assert dts==total;return result
 for mode,data in [('classic',classic()),('third',classic(True)),('fragmented',fragmented()),('hybrid',fragmented(True))]:
  add('adpcm-'+mode,data,1008,108000000,0)
  add('adpcm-'+mode+'-edits',edit_movie(data,[(3,-1),(80,8),(2,-1),(80,8)]),1608,165000000,24)
 files['manifest.json']=(json.dumps(dict(provenance='Owned arithmetic PCM,authored MACE bytes and ADPCM predictor blocks with neutral VP9;classic/reordered/fragmented/hybrid valid movies and silence/repeated edits. No private media/parameters,FFmpeg,process or network.',cases=records,sha256={n:hashlib.sha256(b).hexdigest()for n,b in files.items()}),indent=2)+'\n').encode();return files
if __name__=='__main__':
 OUT.mkdir(parents=True,exist_ok=True)
 for n,b in generate().items():(OUT/n).write_bytes(b)
