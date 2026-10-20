"""Own opaque ALAC cookie compatibility wrappers around authored codec/video data."""
import json,hashlib,struct
from pathlib import Path
import generate_alac_multichannel_fixtures as g
from generate_audio_resample_window_fixture import ebml,word
from generate_vp9_chroma import frame
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/alac-cookie-wrappers'
def movie(cookie,channels,packets):
 video=ebml('ae',g.u('d7',1)+g.u('73c5',1)+g.u('83',1)+ebml('86',b'V_VP9')+ebml('e0',g.u('b0',16)+g.u('ba',16)))
 audio=ebml('ae',g.u('d7',2)+g.u('73c5',2)+g.u('83',2)+ebml('86',b'A_ALAC')+ebml('63a2',cookie)+ebml('e1',ebml('b5',struct.pack('>d',48000))+g.u('9f',channels)))
 blocks=g.u('e7',0)
 for i,payload in enumerate(packets):blocks+=ebml('a3',b'\x81'+struct.pack('>hB',i*1000,128)+frame(8,'420',16,16))+ebml('a3',b'\x82'+struct.pack('>hB',i*333,128)+payload)
 return ebml('1a45dfa3',ebml('4282',b'matroska')+g.u('4287',3)+g.u('4285',2))+ebml('18538067',ebml('1549a966',g.u('2ad7b1',1000))+ebml('1654ae6b',video+audio)+ebml('1f43b675',blocks))
def generate():
 files={};cases=[];invalid=[]
 base=json.loads(g.generate()['manifest.json'])
 for c in base['cases']:
  if c['mode']not in ['raw','rice']:continue
  channels=c['channels'];depth=c['depth'];tag,order,mask,groups=g.LAYOUTS[channels]
  cookie=bytearray(24);cookie[:4]=word(16);cookie[5:10]=bytes([depth,40,10,14,channels]);cookie[20:24]=word(48000)
  if '-named'in c['file']:cookie+=word(24)+b'chan'+word(0,(tag<<16)|channels,0,0)
  stats=dict(ordinary=0,escapes=0,runs=0,run_samples=0,resumed_run_samples=0)
  packets=[g.packet(depth,groups,i*16,channels)if c['mode']=='raw'else g.compressed_packet(depth,groups,i*16,channels,'rice',stats)for i in range(2)]
  for style in ['atom','atom-terminated','format-atom','format-fixed-terminated','atom-fixed']:
   wrapped=word(36 if 'fixed'in style else 12+len(cookie))+b'alac'+word(0)+cookie
   if style.startswith('format'):wrapped=word(12)+b'frma'+b'alac'+wrapped
   if 'terminated'in style:wrapped+=word(8,0)
   name=c['file'][:-4]+'-'+style
   files[name+'.mkv']=movie(wrapped,channels,packets);files[name+'.m4a']=g.mp4(wrapped,channels,depth,packets)
   cases.append(dict(c,file=name+'.mkv',wrapper=style))
 # Simple valid stereo packets accompany malformed legacy headers.
 cookie=bytearray(24);cookie[:4]=word(16);cookie[5:10]=bytes([16,40,10,14,2]);cookie[20:24]=word(48000)
 packets=[g.packet(16,[2],i*16,2)for i in range(2)]
 good=word(36)+b'alac'+word(0)+cookie
 for name,wrapped,error in [
  ('format-size',word(11)+b'frma'+b'alac'+good,'invalid ALAC legacy format atom'),
  ('format-type',word(12)+b'frma'+b'mp4a'+good,'invalid ALAC legacy format atom'),
  ('atom-size',word(35)+good[4:],'invalid ALAC legacy configuration atom'),
  ('atom-truncated',word(60)+good[4:],'truncated ALAC legacy configuration atom'),
  ('atom-flags',good[:8]+word(1)+good[12:],'invalid ALAC legacy configuration atom'),
  ('terminator',good+word(8,1),'invalid ALAC legacy terminator'),
  ('cookie-rate',good[:32]+word(44100),'ALAC cookie and track sample rates disagree'),
  ('cookie-zero-rate',good[:32]+word(0),'ALAC cookie and track sample rates disagree'),
  ('cookie-channels',good[:21]+bytes([3])+good[22:],"does not match its cookie"),
 ]:
  name='invalid-'+name+'.mkv';files[name]=movie(wrapped,2,packets);invalid.append(dict(file=name,error=error))
 for version in [1,255]:
  bad=bytearray(cookie);bad[4]=version
  for style,payload in [('raw',bad),('wrapped',word(12)+b'frma'+b'alac'+word(36)+b'alac'+word(0)+bad+word(8,0))]:
   name=f'invalid-version-{version}-{style}.mkv'
   files[name]=movie(payload,2,packets)
   invalid.append(dict(file=name,error='unsupported ALAC compatible version'))
 files['manifest.json']=(json.dumps(dict(provenance='Own ALAC raw/Rice frames and neutral VP9, opaque modern cookies in documented compatibility wrappers. Includes named/discrete channels, sized/fixed atoms and optional format/terminator atoms. No private media or foreign encoder.',cases=cases,invalid=invalid,files={n:hashlib.sha256(v).hexdigest()for n,v in files.items()}),indent=2)+'\n').encode();return files
if __name__=='__main__':
 OUT.mkdir(exist_ok=True)
 for n,b in generate().items():(OUT/n).write_bytes(b)
