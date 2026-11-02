"""Authored QuickTime G711 v1 movies and independent PCM segment-level oracle."""
import struct,json,hashlib
from pathlib import Path
from mp4_audio_witness import movie
from generate_vp9_chroma import atom,word
from generate_g711_fixtures import level
from generate_mp4_fragment_description_regressions import atoms
from generate_mp4_variable_rate_edits import edit_movie
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/mp4-g711-seek'
SCALE=14112000

def uniform_movie(data,frames,ch,tick):
 def rewrite(body,audio=False):
  out=b''
  for k,b in atoms(body):
   if k==b'trak':b=rewrite(b,b'soun'in b)
   elif k in [b'moov',b'mdia',b'minf',b'stbl']:b=rewrite(b,audio)
   elif audio and k==b'stts':b=word(0,1,frames,tick)
   elif audio and k==b'stsz':b=word(0,ch,frames)
   elif audio and k==b'stsc':b=word(0,1,1,frames,1)
   elif audio and k==b'stco':b=word(0,1)+b[8:12]
   out+=atom(k,b)
  return out
 return rewrite(data)

def generate():
 files={};cases=[]
 for law,tag in [('alaw',b'alaw'),('mulaw',b'ulaw')]:
  for rate,ch in [(8000,1),(48000,2),(44100,2)]:
   entry=bytearray(44);entry[6:8]=struct.pack('>H',1);entry[8:10]=struct.pack('>H',1);entry[16:20]=struct.pack('>HH',ch,16);entry[20:22]=struct.pack('>h',-2);entry[24:28]=word(rate<<16)
   # v1 redefined sample tables explicitly bound compressed packet time/bytes.
   # Optional packet geometry fields are unused (all zero).
   packets=[];expected=b'';cursor=0
   for count in [128,257,383]:
    raw=bytes((cursor+n+c*71)%256 for n in range(count)for c in range(ch));packets.append((raw,count));expected+=b''.join(struct.pack('<f',level(law,x)/32768)for x in raw);cursor+=count
   base=f'{law}-{rate}-{ch}ch';files[base+'.f32le']=expected;stride=ch*4;edited_pcm=bytes(32*stride)+expected[32*stride:512*stride]+bytes(16*stride)+expected[128*stride:384*stride];files[base+'-edits.f32le']=edited_pcm
   encoded=b''.join(raw for raw,_ in packets)
   def chunk(tag,body):return tag+struct.pack('<I',len(body))+body+bytes(len(body)%2)
   fmt=struct.pack('<HHIIHHH',6 if law=='alaw'else 7,ch,rate,rate*ch,ch,8,0);wave=b'WAVE'+chunk(b'fmt ',fmt)+chunk(b'fact',struct.pack('<I',cursor))+chunk(b'data',encoded);files[base+'.wav']=b'RIFF'+struct.pack('<I',len(wave))+wave
   for mode in ['classic','fragmented','hybrid','uniform']:
    units=[(encoded[i*ch:(i+1)*ch],1)for i in range(cursor)]if mode=='uniform'else packets
    catalog=[dict(entry=atom(tag,entry),packets=units,rate=rate,channels=ch)]
    data,total,frames,_=movie(catalog,[0],SCALE,'classic'if mode=='uniform'else mode)
    if mode=='uniform':data=uniform_movie(data,cursor,ch,SCALE//rate)
    size=int.from_bytes(data[:4],'big');brand=bytearray(data[8:size]);brand[:4]=b'qt  'if mode in ['classic','uniform']else b'iso5';brand[8:12]=b'qt  ';data=atom(b'ftyp',brand)+data[size:]
    for edited in [False,True]:
     name=base+'-'+mode+('-edits'if edited else'')+'.mov';ticks=SCALE//rate;native=784 if edited else frames;duration=native*ticks
     files[name]=edit_movie(data,[(32*ticks,-1),(480*ticks,32*ticks),(16*ticks,-1),(256*ticks,128*ticks)],clock=SCALE)if edited else data
     end=duration*10**9//SCALE;cases.append(dict(file=name,oracle=base+('-edits'if edited else'')+'.f32le',wave=base+'.wav',edited=edited,uniform=mode=='uniform',codec=tag.decode(),rate=rate,channels=ch,frames=native,end_num=duration,end_den=SCALE,queries_ns=sorted(set([0,1,500001,1000001,15999999,16000000,16000001,end-1,end,end+1]))))
 # Explicit malformed packet: one orphan encoded channel byte, not private data.
 bad_entry=bytearray(44);bad_entry[6:8]=struct.pack('>H',1);bad_entry[8:10]=struct.pack('>H',1);bad_entry[16:20]=struct.pack('>HH',2,16);bad_entry[20:22]=struct.pack('>h',-2);bad_entry[24:28]=word(48000<<16)
 bad_codes=bytes(i%256 for i in range(256));bad=[dict(entry=atom(b'alaw',bad_entry),packets=[(bad_codes[:-1],128)],rate=48000,channels=2)]
 files['orphan-channel.mov']=movie(bad,[0],SCALE)[0]
 files['manifest.json']=(json.dumps(dict(provenance='Authored all256 G711 code values,independent segment-level PCM oracle,neutral VP9 video. QuickTime v1 compressionID-2 redefined sample tables;16-bit uncompressed sampleSize,unused optional packet geometry0. Dense,uniform,fragmented,hybrid,silence and repeated edits. No private media/parameters,FFmpeg or network.',specification=['https://developer.apple.com/documentation/quicktime-file-format/sound_sample_descriptions','https://developer.apple.com/documentation/quicktime-file-format/sound_sample_description_version_1'],cases=cases,sha256={n:hashlib.sha256(b).hexdigest()for n,b in files.items()}),indent=2)+'\n').encode();return files
if __name__=='__main__':
 OUT.mkdir(parents=True,exist_ok=True)
 for n,b in generate().items():(OUT/n).write_bytes(b)
