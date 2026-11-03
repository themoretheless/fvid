"""Author version1 ALAC/EAC3 configuration offsets with unchanged version0 coded media."""
from pathlib import Path
import json,struct
from generate_mp4_audio_clock_regressions import generate as alac
from generate_eac3_dependent import packet
from generate_eac3_mp4_programmes import movie,transform
from generate_mp4_enabled_track_regressions import boxes
from generate_audio_resample_window_fixture import atom,word
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/mp4-v1-coded-config'
def generate():
 a=alac();files={'alac.f32le':a['expected.f32le']};cases=[];controls=[];bad=[]
 raw,_=packet(2,False,0,0,'mixed',None,False,blocks_override=6,gains=[64]+[None]*5,compact=True)
 sources=[(f'alac-t{clock}',a[f'alac-{clock}.mp4'],'alac',8000,1,160,480)for clock in [8000,1000,48000]]
 e=movie([raw]*3,[(2,False,0,0)],48000)
 def remove_config(data,audio=False):
  out=b''
  for k,b in boxes(data):
   active=audio or(k==b'trak'and b'ec-3'in b)
   if k in [b'moov',b'trak',b'mdia',b'minf',b'stbl']:b=remove_config(b,active)
   elif active and k==b'stsd':tag,entry=next(boxes(b[8:]));b=b[:8]+atom(tag,entry[:28])
   out+=atom(k,b)
  return out
 for clock in [48000,1000]:
  original=e if clock==48000 else transform(e,clock=True)
  for config in [True,False]:sources.append((f'ec3-t{clock}-'+('dec3'if config else'packet-probe'),original if config else remove_config(original),'ec-3',48000,2,1536,4608))
 for stem,source,codec,rate,ch,samples,frames in sources:
  # Preserve ftyp length/offsets while declaring the QuickTime sound layout.
  parts=list(boxes(source));source=b''.join(atom(k,b'qt  '+b[4:8]+b'qt  '*((len(b)-8)//4)if k==b'ftyp'else b)for k,b in parts)
  control=stem+'-v0.mov';files[control]=source;controls.append(dict(file=control,codec=codec,frames=frames,rate=rate,channels=ch))
  for mode in ['packet-fields','unspecified-fields']:
   def rewrite(data,audio=False,short=None):
    out=b''
    for k,b in boxes(data):
     active=audio or(k==b'trak'and(codec.encode()in b))
     if k in [b'moov',b'trak',b'mdia',b'minf',b'stbl']:b=rewrite(b,active,short)
     elif active and k==b'stsd':
      tag,entry=next(boxes(b[8:]));header=bytearray(entry[:28]);header[8:10]=struct.pack('>H',1);header[20:22]=struct.pack('>h',-2);extension=word(samples,0,0,2)if mode=='packet-fields'else bytes(16);body=header+extension+entry[28:]
      if short is not None:body=body[:short]
      b=b[:8]+atom(tag,body)
     out+=atom(k,b)
    return out
   name=stem+'-v1-'+mode+'.mov';files[name]=rewrite(source);cases.append(dict(file=name,control=control,codec=codec,frames=frames,rate=rate,channels=ch,config=('dec3'if stem.endswith('dec3')else 'none')if codec=='ec-3'else'alac'))
   if mode=='packet-fields'and stem in ['alac-t8000','ec3-t48000-dec3']:
    for size in [28,32,40,43]:
     name=stem+f'-short{size}.mov';files[name]=rewrite(source,short=size);bad.append(name)
 files['manifest.json']=(json.dumps(dict(provenance='Authored60ms mono raw-ALAC ramps and96ms stereo EAC3 spectra with three coded packets, neutral VP9 pictures, QT brand, v0/v1 header-only companions. Correct configuration bytes unchanged after the required16-byte version1 extension. No private media, encoder or network.',cases=cases,controls=controls,bad=bad),indent=2)+'\n').encode();return files
if __name__=='__main__':
 OUT.mkdir(parents=True,exist_ok=True)
 for n,b in generate().items():(OUT/n).write_bytes(b)
