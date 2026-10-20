#!/usr/bin/env python3
"""Independent finite-convolution oracle for ISO MPEG synthesis.
Saved subband inputs and float64 PCM; no decoder executable or network.
"""
import math,re,struct,json,hashlib
from pathlib import Path
R=Path(__file__).resolve().parents[1]
D=R/'tests/fixtures/playback-errors/mpeg-synthesis';D.mkdir(exist_ok=True)
text=(R/'crates/fvid-codecs/src/mpeg_synthesis_window.rs').read_text().split('= [',1)[1]
window=[int(v)for v in re.findall(r'-?\d+',text)];assert len(window)==512
records=[]
def emit(name,inputs):
 # Time-domain convolution: no delay-line shuffle or synthesis matrix storage.
 output=[]
 for t in range(len(inputs)):
  for j in range(32):
   value=0.
   for lag in range(min(16,t+1)):
    q=lag//2;row=j+(32 if lag%2 else 0)
    weight=window[64*q+32*(lag%2)+j]/65536.
    value+=weight*sum(v*math.cos(math.pi*(16+row)*(2*k+1)/64.)for k,v in enumerate(inputs[t-lag]))
   output.append(value)
 raw=b''.join(struct.pack('<d',v)for row in inputs for v in row)
 expected=b''.join(struct.pack('<d',v)for v in output)
 (D/f'{name}.bands').write_bytes(raw);(D/f'{name}.f64le').write_bytes(expected)
 records.append(dict(name=name,slots=len(inputs),input_sha256=hashlib.sha256(raw).hexdigest(),pcm_sha256=hashlib.sha256(expected).hexdigest()))
for band in range(32):
 inputs=[[float(k==band and t==0)for k in range(32)]for t in range(32)]
 emit(f'impulse-{band}',inputs)
emit('alternating',[[(-1. if(t+k)%2 else 1.)/64 for k in range(32)]for t in range(48)])
emit('changing',[[(((t*73+k*31+19)%257)-128)/4096 for k in range(32)]for t in range(48)])
(D/'manifest.json').write_text(json.dumps(dict(reference='ISO/IEC 11172-3 Annex A.2, Table B.3',cases=records,video_companion='../g711-alaw-2ch.avi'),indent=2)+'\n')
print(len(records),'MPEG synthesis cases')
