"""Owned AC-3 BSID9/10 quiet spectra and independent full-rate PCM controls."""
import json
from pathlib import Path
from generate_eac3_playback import BitWriter
from generate_ac3_rf_compression import crc_frame,SIZE,RATE
from generate_eac3_reduced_allocation import EXP,bap,mantissas
from generate_eac3_bsid import movie
from generate_eac3_aht_lfe import audiovisual
from generate_eac3_mp4_presentation import rewrite
from generate_mp4_enabled_track_regressions import boxes
from generate_audio_resample_window_fixture import atom
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/ac3-reduced-compatibility'
def packet(row,bsid,snr,values):
 shift=max(0,bsid-8);codes=bap(row,shift,snr);decoded=[]
 w=BitWriter().put(0x0b77,16).zeros(16).put(row,2).put(36,6).put(bsid,5).zeros(3).put(1,3).put(0,1).put(31,5).zeros(8)
 for b in range(6):
  w.zeros(3) # long window, dither off, no dynamic-range word
  w.put(int(b==0),1)
  if b==0:w.put(0,1) # coupling off
  w.put(1 if b==0 else 0,2)
  if b==0:
   w.put(0,6).put(15,4).put(124,7).put(117,7)
   for _ in range(22):w.put(62,7)
   w.zeros(2)
  w.put(int(b==0),1)
  if b==0:w.put(3,2).put(3,2).put(3,2).put(0,2).put(7,3)
  w.put(int(b==0),1)
  if b==0:w.put(snr//64+15,6).put((snr//4)%16,4).put(4,3)
  w.zeros(2) # no delta allocation or skipped bytes
  decoded.append(mantissas(w,codes,values[b]))
 return crc_frame(w.to_bytes(SIZE[row])),decoded

def mp4(packets,row,bsid):
 config=int(f'{row:02b}{bsid:05b}{0:03b}{1:03b}{0:01b}{18:05b}'+'00000',2).to_bytes(3,'big')
 data=rewrite(audiovisual(packets),config,RATE[row]>>max(0,bsid-8))
 def convert(data):
  out=b''
  for k,b in boxes(data):
   if k in [b'moov',b'trak',b'mdia',b'minf',b'stbl']:b=convert(b)
   elif k==b'stsd' and b[12:16]==b'ec-3':
    entry=bytearray(b[16:44]);entry[16:18]=(1).to_bytes(2,'big')
    b=b[:8]+atom(b'ac-3',entry+atom(b'dac3',config))
   out+=atom(k,b)
  return out
 return convert(data)

def generate():
 files={};cases=[]
 values=[[0. if i<8 else .5*((i+b)%3-1)for i in range(73)]for b in range(6)]
 for row in range(3):
  for bsid in [0,8,9,10]:
   rate=RATE[row]>>max(0,bsid-8);name=f'bsid{bsid}-{rate}'
   raw,decoded=packet(row,bsid,1088,values)
   control,reference=packet(row,8,3132,decoded)
   error=max(4*sum(abs(a-b)*2**-e for a,b,e in zip(x,y,EXP))for x,y in zip(decoded,reference))
   files[name+'.ac3']=raw*3;files[name+'-control.ac3']=control*3
   files[name+'.mkv']=movie([raw]*3,channels=1,rate=rate,codec='A_AC3')
   files[name+'-alias.mkv']=movie([raw]*3,channels=1,rate=rate,codec=f'A_AC3/BSID{bsid}' if bsid in [9,10] else 'A_AC3')
   files[name+'.mp4']=mp4([raw]*3,row,bsid)
   le=b''.join(raw[i:i+2][::-1]for i in range(0,len(raw),2))
   files[name+'-le.ac3']=le*3
   cases.append(dict(name=name,bsid=bsid,rate=rate,base_rate=RATE[row],frames=4352,error_bound=error))
 files['manifest.json']=(json.dumps(dict(cases=cases,provenance='Owned mono quiet spectra and independent full-rate high-precision PCM controls; public normative allocation tables. Neutral VP9 videos. BSID9/10 are de facto compatibility, not ATSC modes. No private media, external encoder, subprocess or network.'),indent=2)+'\n').encode()
 return files
if __name__=='__main__':
 OUT.mkdir(parents=True,exist_ok=True)
 for n,b in generate().items():(OUT/n).write_bytes(b)
