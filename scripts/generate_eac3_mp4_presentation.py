"""Owned 8/10-channel E-AC-3 MP4, six-block samples and placeholder channelcount=2."""
import json, struct
from pathlib import Path
from generate_eac3_presentation import generate as presentations
from generate_eac3_aht_lfe import audiovisual
from generate_mp4_enabled_track_regressions import boxes
from generate_audio_resample_window_fixture import atom
from generate_eac3_playback import BitWriter
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/eac3-mp4-presentation'
def rewrite(data, config, sample_rate, audio=False):
 out=b''
 for kind,body in boxes(data):
  if kind in [b'moov',b'trak',b'mdia',b'minf',b'stbl']:
   selected=(b'ec-3' in body) if kind==b'trak' else audio
   body=rewrite(body,config,sample_rate,selected)
  elif kind==b'mdhd' and audio:
   body=bytearray(body);body[12:16]=struct.pack('>I',sample_rate)
  elif kind==b'tkhd' and audio:
   body=bytearray(body);body[20:24]=struct.pack('>I',(4608*1000+sample_rate-1)//sample_rate)
  elif kind==b'mvhd' and sample_rate!=48000:
   body=bytearray(body);body[16:20]=struct.pack('>I',max(100,(4608*1000+sample_rate-1)//sample_rate))
  elif kind==b'stsd' and body[12:16]==b'ec-3':
   entry=bytearray(body[16:44]);entry[16:18]=struct.pack('>H',2);entry[24:28]=struct.pack('>I',sample_rate<<16)
   body=body[:8]+atom(b'ec-3',entry+atom(b'dec3',config))
  out+=atom(kind,body)
 return out

def generate():
 source=presentations();cases=[];files={}
 for c in json.loads(source['manifest.json'])['cases']:
  if c['wave_mask'] is None:continue
  bundle=b''.join(source[p['raw']]for p in c['components'])
  # Two three-block programmes form each six-block MP4 sample.
  rate=(len(bundle)*2*8*c['rate']+1535999)//1536000
  locations=2 | (64 if c['channels']==10 else 0)
  dependents=len(c['components'])-1
  w=BitWriter().put(rate,13).put(0,3).put({48000:0,44100:1}[c['rate']],2).put(16,5).zeros(5).put(7,3).put(1,1).zeros(3).put(dependents,4).put(locations,9)
  assert len(w.bits)==48
  config=bytes(sum(w.bits[i+j]<<(7-j)for j in range(8))for i in range(0,48,8))
  name=c['name']+'.mp4';files[name]=rewrite(audiovisual([bundle*2]*3),config,c['rate'])
  cases.append(dict(file=name,source=c['file'],channels=c['channels'],mask=c['wave_mask'],samples=4608,rate=c['rate']))
 # Two valid tracks may share encoded sample storage; descriptions must be
 # correct before either is selected. Only the duplicated track ID differs.
 original=files[cases[0]['file']];roots=list(boxes(original));result=b''
 for kind,body in roots:
  if kind==b'moov':
   children=list(boxes(body));audio=next(b for k,b in children if k==b'trak' and b'ec-3' in b)
   clone=b''
   for k,b in boxes(audio):
    if k==b'tkhd':b=bytearray(b);b[12:16]=struct.pack('>I',3)
    clone+=atom(k,b)
   revised=b''
   for k,b in children:
    if k==b'mvhd':b=bytearray(b);b[96:100]=struct.pack('>I',4)
    revised+=atom(k,b)
   body=revised+atom(b'trak',clone)
  result+=atom(kind,body)
 files['two-expanded-audio-tracks.mp4']=result
 files['manifest.json']=(json.dumps(dict(cases=cases,two_track='two-expanded-audio-tracks.mp4'),indent=2)+'\n').encode()
 return files
if __name__=='__main__':
 OUT.mkdir(parents=True,exist_ok=True)
 for name,data in generate().items():(OUT/name).write_bytes(data)
