#!/usr/bin/env python3
"""Offline authored long MP3/video chains for checkpoint navigation regression.
Reuses FVid-authored compressed frames; no private media, encoder or network.
"""
from pathlib import Path
import struct,json,hashlib
from generate_audio_resample_window_fixture import ebml
from generate_vp9_chroma import frame
R=Path(__file__).resolve().parents[1];D=R/'tests/fixtures/playback-errors/mp3-checkpoint';D.mkdir(exist_ok=True)
S=D.parent/'mp3-decoder';meta=json.loads((S/'manifest.json').read_text())['cases'];files={};cases=[]
for name,rate,backwards in [('long',48000,False),('fractional',44100,False),('backwards',48000,True)]:
 source=(S/f'{rate}-1ch.mp3').read_bytes();sizes=next(c['sizes']for c in meta if c['name']==f'{rate}-1ch');at=0;packets=[]
 for size in sizes:packets.append(source[at:at+size]);at+=size
 packets*=40
 if not backwards:files[name+'.mp3']=b''.join(packets)
 header=ebml('1a45dfa3',ebml('4282',b'matroska')+ebml('4287',b'\3')+ebml('4285',b'\2'))
 info=ebml('1549a966',ebml('2ad7b1',(1000000).to_bytes(3,'big')))
 video=ebml('ae',ebml('d7',b'\1')+ebml('73c5',b'\1')+ebml('83',b'\1')+ebml('86',b'V_VP9')+ebml('23e383',round(1152*1e9/rate).to_bytes(4,'big'))+ebml('e0',ebml('b0',b'\x10')+ebml('ba',b'\x10')))
 audio=ebml('ae',ebml('d7',b'\2')+ebml('73c5',b'\2')+ebml('83',b'\2')+ebml('86',b'A_MPEG/L3')+ebml('e1',ebml('b5',struct.pack('>d',rate))+ebml('9f',b'\1')))
 blocks=ebml('e7',b'\0')
 for i,p in enumerate(packets):
  time=round(i*1152*1000/rate)
  blocks+=ebml('a3',b'\x81'+struct.pack('>h',time)+b'\x80'+frame(8,'420',16,16))
  audio_time=time+(3000 if backwards and 40<=i<80 else -1000 if backwards and i>=80 else 0)
  blocks+=ebml('a3',b'\x82'+struct.pack('>h',audio_time)+b'\x80'+p)
 files[name+'.mkv']=header+ebml('18538067',info+ebml('1654ae6b',video+audio)+ebml('1f43b675',blocks))
 cases.append({'name':name,'frames':160,'rate':rate,'backwards':backwards})
for name,data in files.items():(D/name).write_bytes(data)
(D/'manifest.json').write_text(json.dumps({'cases':cases,'reference':'Checkpoint suffix compared against uninterrupted owned PCM; not a new codec PCM oracle','files':{name:hashlib.sha256(data).hexdigest()for name,data in files.items()}},indent=2)+'\n')
