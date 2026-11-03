"""Author short MP4 track-order budget controls, with explicit opaque codecs."""
from pathlib import Path
from itertools import permutations
import json,struct
from generate_mp4_audio_clock_regressions import generate as clocks
from generate_mp4_enabled_track_regressions import boxes,canonical_movie
from generate_audio_resample_window_fixture import atom,word
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/mp4-track-budget-order'
def generate():
 source=clocks()['pcm-1000.mp4'];top=list(boxes(source));movie=list(boxes(dict(top)[b'moov']));tracks=[b for k,b in movie if k==b'trak'];assert len(tracks)==2
 def opaque(data,ident,active=False):
  out=b''
  for k,b in boxes(data):
   if k in [b'mdia',b'minf',b'stbl']:b=opaque(b,ident)
   elif k==b'tkhd':b=b[:12]+word(ident)+b[16:]
   elif k==b'stsd':tag,payload=next(boxes(b[8:]));assert tag==b'sowt';b=b[:8]+atom(f'z{ident:03d}'.encode(),payload)
   out+=atom(k,b)
  return out
 options={'v':tracks[0],'a':opaque(tracks[1],2),'b':opaque(tracks[1],3)}
 files={};cases=[]
 def write(name,chosen,unsupported):
  moov=atom(b'moov',b''.join(atom(k,canonical_movie(b,4)if k==b'mvhd'else b)for k,b in movie if k!=b'trak')+b''.join(atom(b'trak',t)for t in chosen))
  files[name]=b''.join(moov if k==b'moov'else atom(k,b)for k,b in top);cases.append(dict(file=name,totalTracks=len(chosen),supportedTracks=len(chosen)-unsupported,unsupportedTracks=unsupported))
 write('video-only.mp4',[tracks[0]],0)
 write('known-av.mp4',tracks,0)
 for letters in [('v','a'),('v','a','b')]:
  for order in permutations(letters):write('order-'+''.join(order)+'.mp4',[options[k]for k in order],len(order)-1)
 files['frame.yuv16']=struct.pack('<384H',*([128]*384))
 files['manifest.json']=(json.dumps(dict(provenance='Authored60ms16x16 neutral VP9 and optional8kHz mono PCM. Opaque z002/z003 descriptions are explicitly unsupported; their common headers, references, IDs and tables remain well formed. Only trak order/count changes; media offsets and payloads stay unchanged. No private media, parameters, encoder or network.',cases=cases),indent=2)+'\n').encode()
 return files
if __name__=='__main__':
 OUT.mkdir(parents=True,exist_ok=True)
 for n,b in generate().items():(OUT/n).write_bytes(b)
