"""Owned E-AC-3 AHT LFE / mixed-plane decode and speaker assignment witnesses."""
import json,struct
from pathlib import Path
from generate_eac3_playback import BitWriter,patch_crc
from generate_eac3_aht import spectrum,conventional
from generate_eac3_bsid import movie
from generate_mp4_enabled_track_regressions import boxes,canonical_movie,rewrite_track,sample_track
from generate_vp9_chroma import frame,mp4
from generate_audio_resample_window_fixture import atom,word
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/eac3-aht-lfe'
def audiovisual(packets):
 base=dict(boxes(mp4([frame(8,'420',16,16)]*5,8,'420',(16,16),sync_samples=[1,2,3,4,5])))
 ftyp=atom(b'ftyp',base[b'ftyp']);payload=base[b'mdat'];meta=dict(boxes(base[b'moov']))
 def duration(track,ticks):
  out=b''
  for k,b in boxes(track):
   if k==b'tkhd':b=bytearray(b);b[20:24]=word(ticks)
   out+=atom(k,b)
  return out
 video=atom(b'trak',duration(rewrite_track(meta[b'trak'],1,3,len(ftyp)+8,False),100))
 entry=bytearray(28);entry[6:8]=b'\0\1';entry[16:20]=struct.pack('>HH',3,16);entry[24:28]=word(48000<<16)
 # ETSI TS 102 366 F.6: one independent 48 kHz stereo+LFE stream,
 # 4096 bytes / 1536 samples = 1024 kbit/s, no dependent substreams.
 config=BitWriter().put(1024,13).put(0,3).put(0,2).put(16,5).zeros(5).put(2,3).put(1,1).zeros(8)
 assert len(config.bits)==40
 payload_config=bytes(sum(config.bits[i+j]<<(7-j)for j in range(8))for i in range(0,40,8))
 entry+=atom(b'dec3',payload_config)
 audio=sample_track(2,3,b'soun',b'ec-3',entry,packets,len(ftyp)+8+len(payload),48000,1536)
 audio=atom(b'trak',duration(dict(boxes(audio))[b'trak'],96))
 mvhd=canonical_movie(meta[b'mvhd'],3);mvhd[12:20]=word(1000,100)
 return ftyp+atom(b'mdat',payload+b''.join(packets))+atom(b'moov',atom(b'mvhd',mvhd)+video+audio)
def packet(mask,converted,basis,aht=True,*,zero=False,flags=None,exponent=10):
 w=BitWriter();w.put(0x0b77,16).put(2 if converted else 0,2).put(0,3).put(2047,11)
 w.put(0,2).put(3,2).put(2,3).put(1,1).put(16,5).put(31,5).zeros(3)
 if converted:w.put(20,6)
 w.put(0,1).put(1,1).put(int(aht),1).put(0,2)
 w.zeros(2).put(int(flags is not None),1).zeros(5).zeros(6)
 for b in range(6):w.put(3 if b==0 else 0,2).put(3 if b==0 else 0,2)
 w.put(1,1).zeros(5) # LFE D15 then reuse
 if not converted:w.zeros(10)
 if aht:
  for ch in range(3):w.put((mask>>ch)&1,1)
 w.put(0 if zero else 63,6).put(0 if zero else 15,4).put(0,1)
 for b in range(6):
  if flags is not None:w.put(flags[b][0],1).put(flags[b][1],1)
  w.zeros(2)
  if b==0:
   w.zeros(16)
   for ch in range(2):
    w.put(exponent,4)
    for _ in range(6):w.put(62,7)
    w.zeros(2)
   w.put(exponent,4).put(62,7).put(62,7) # 7 LFE exponents, no gainrng
  else:w.put(0,1)
  if not converted:w.put(0,1)
  for ch in range(3):
   active=aht and mask&(1<<ch)
   if active:
    if b!=0:continue
    w.put(0,2)
   if zero:continue
   for bin in range(7 if ch==2 else 73):
    values=spectrum(bin,ch,basis)
    if active:
     for q in values:w.put(q&65535,16)
    else:w.put(round(conventional(values,b))&65535,16)
 return patch_crc(w.to_bytes(4096))
def generate():
 files={};cases=[]
 for converted in [False,True]:
  for mask in range(8):
   for basis in [None,3,'mixed']:
    name=f't{2 if converted else 0}-m{mask}-{basis if basis is not None else "dc"}'
    raw=packet(mask,converted,basis);control=packet(0,converted,basis,False)
    l1=max(sum(abs(conventional(spectrum(i,ch,basis),b)-round(conventional(spectrum(i,ch,basis),b)))for i in range(7 if ch==2 else 73))for ch in range(3)for b in range(6))
    for suffix,data in [('.ec3',raw*3),('-control.ec3',control*3),('.mkv',movie([raw]*3,channels=3)),('-control.mkv',movie([control]*3,channels=3)),('.mp4',audiovisual([raw]*3)),('-control.mp4',audiovisual([control]*3))]:files[name+suffix]=data
    cases.append(dict(name=name,file=name+'.ec3',control=name+'-control.ec3',video=name+'.mkv',control_video=name+'-control.mkv',mp4=name+'.mp4',control_mp4=name+'-control.mp4',blocks=6,channels=3,mask=mask,quantization_bound=4*l1/(32768*1024),exact=basis in [None,3]))
 files['manifest.json']=(json.dumps(dict(cases=cases),indent=2)+'\n').encode()
 return files
if __name__=='__main__':
 OUT.mkdir(parents=True,exist_ok=True)
 for n,b in generate().items():(OUT/n).write_bytes(b)
