"""Author v0/v1 track-ID namespace controls with unchanged short coded media."""
from pathlib import Path
import json,struct
from generate_mp4_track_budget_order import generate as controls
from generate_mp4_enabled_track_regressions import boxes
from generate_audio_resample_window_fixture import atom,word
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/mp4-opaque-track-ids'
def generate():
 base=controls();manifest=json.loads(base['manifest.json']);files={'frame.yuv16':base['frame.yuv16']};good=[];bad=[]
 def header(data,version,old=None,new=None):
  out=b''
  for k,b in boxes(data):
   if k in [b'moov',b'trak']:b=header(b,version,old,new)
   elif k==b'tkhd':
    assert b[0]==0
    if old is not None and int.from_bytes(b[12:16],'big')==old:b=b[:12]+word(new)+b[16:]
    if version:b=bytes([1])+b[1:4]+bytes(16)+b[12:20]+struct.pack('>Q',int.from_bytes(b[20:24],'big'))+b[24:]
   out+=atom(k,b)
  return out
 for v in [0,1]:
  for c in manifest['cases']:
   n=f'v{v}-'+c['file'];files[n]=header(base[c['file']],v);good.append(dict(c,file=n))
   if c['file'].startswith('order-'):
    prior=2 if c['totalTracks']==2 else 3;assigned=1 if prior==2 else 2
    n=f'v{v}-duplicate-'+c['file'];files[n]=header(base[c['file']],v,prior,assigned);bad.append(dict(file=n,previouslyAccepted=True,totalTracks=c['totalTracks']))
   elif c['file']=='known-av.mp4':
    n=f'v{v}-duplicate-known.mp4';files[n]=header(base[c['file']],v,2,1);bad.append(dict(file=n,previouslyAccepted=False,totalTracks=2))
 files['manifest.json']=(json.dumps(dict(provenance='Authored60ms neutral VP9 with optional known PCM or opaque z002/z003 sound tracks. Distinct-ID controls and duplicate-ID companions retain payloads, references, sample tables and offsets; tkhd v0/v1 layouts are authored explicitly. No private media, encoder, random mutation audit or network.',cases=good,bad=bad),indent=2)+'\n').encode()
 return files
if __name__=='__main__':
 OUT.mkdir(parents=True,exist_ok=True)
 for n,b in generate().items():(OUT/n).write_bytes(b)
