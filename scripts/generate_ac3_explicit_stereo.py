"""Owned C+LFE witnesses: native two-channel PCM is not L/R stereo."""
import json
from pathlib import Path
from generate_ac3_rf_compression import authored,crc_frame,RATE
from generate_eac3_bsid import movie
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/ac3-explicit-stereo'
def generate():
 files={};cases=[]
 for rc,base_rate in enumerate(RATE):
  original=authored(1,True,rc,None,None,control=True)
  for bsid,shift in [(8,0),(9,1),(10,2)]:
   raw=bytearray(original);raw[5]=(raw[5]&7)|(bsid<<3);raw=crc_frame(raw)
   name=f'c-lfe-r{base_rate}-b{bsid}';rate=base_rate>>shift
   files[name+'.ac3']=raw*3
   files[name+'.mkv']=movie([raw]*3,channels=2,rate=rate,codec='A_AC3')
   cases.append(dict(name=name,rate=rate,bsid=bsid,frame_bytes=len(raw),raw_frames=4352,container_frames=4608))
 files['manifest.json']=(json.dumps(dict(cases=cases,provenance='Owned distinct C/LFE spectra and neutral VP9; no private media, process, network or external encoder.'),indent=2)+'\n').encode()
 return files
if __name__=='__main__':
 OUT.mkdir(parents=True,exist_ok=True)
 for n,b in generate().items():(OUT/n).write_bytes(b)
