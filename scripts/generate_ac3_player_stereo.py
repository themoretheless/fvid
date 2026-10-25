"""Owned AC3 BSI stereo negotiation and C+LFE source witnesses."""
import json
from pathlib import Path
from generate_ac3_rf_compression import authored,mp4_movie
from generate_ac3_alternate_bsi import replace
from generate_ac3_stereo_downmix import source_frame
from generate_eac3_bsid import movie
from generate_mp4_enabled_track_regressions import boxes
from generate_audio_resample_window_fixture import atom,word
ROOT=Path(__file__).resolve().parents[1]
OUT=ROOT/'tests/fixtures/playback-errors/ac3-player-stereo'
def generate():
 files={};cases=[]
 tone=(ROOT/'tests/fixtures/audio/ac3-51.ac3').read_bytes()
 families=[('51',[tone[i*1792:(i+1)*1792]for i in range(3)],7,True,48000,2560),('single',[source_frame(5)]*3,5,False,32000,3840),('c-lfe',[authored(1,True,0,None,None,control=True)]*3,1,True,48000,2560)]
 for name,original,layout,lfe,rate,size in families:
  mixes=[[1,0,4,7,6],[2,7,6,0,4],None]
  packets=[replace(raw,6,m,None,frame_bytes=size)for raw,m in zip(original,mixes)]
  files[name+'.ac3']=b''.join(packets);channels=[2,1,2,3,3,4,4,5][layout]+int(lfe)
  files[name+'.mkv']=movie(packets,channels=channels,rate=rate,codec='A_AC3')
  files[name+'.mp4']=mp4_movie(packets,layout,lfe,[48000,44100,32000].index(rate),bsid=6)
  cases.append(dict(name=name,channels=channels,rate=rate,layout=layout,lfe=lfe,bsid=6,mixes=mixes))
 # A real coding/rate/layout boundary: 48k 5.1 -> 32k single surround -> 48k.
 roots=dict(boxes(files['51.mp4']));ftyp=atom(b'ftyp',roots[b'ftyp']);video=roots[b'mdat'][:-len(files['51.ac3'])]
 def audio_track(data):return next(b for k,b in boxes(dict(boxes(data))[b'moov'])if k==b'trak'and b'ac-3'in b)
 def table(track):return dict(boxes(dict(boxes(dict(boxes(track))[b'mdia']))[b'minf']))[b'stbl']
 entries=[dict(boxes(table(audio_track(files[n+'.mp4']))))[b'stsd'][8:]for n in ['51','single']]
 packets=[files['51.ac3'][:2560],files['single.ac3'][3840:7680],files['51.ac3'][5120:7680]]
 offsets=[];offset=len(ftyp)+8+len(video)
 for packet in packets:offsets.append(offset);offset+=len(packet)
 stbl=atom(b'stsd',word(0,2)+b''.join(entries))+atom(b'stts',word(0,3,1,32000,1,48000,1,32000))+atom(b'stsc',word(0,3,1,1,1,2,1,2,3,1,1))+atom(b'stsz',word(0,0,3,*map(len,packets)))+atom(b'stco',word(0,3,*offsets))
 def change(data):
  out=b''
  for k,b in boxes(data):
   if k in [b'mdia',b'minf']:b=change(b)
   elif k==b'stbl':b=stbl
   elif k==b'mdhd':b=bytearray(b);b[12:20]=word(1000000,112000)
   elif k==b'tkhd':b=bytearray(b);b[20:24]=word(112)
   out+=atom(k,b)
  return out
 moov=b''
 for k,b in boxes(roots[b'moov']):
  if k==b'trak'and b'ac-3'in b:b=change(b)
  elif k==b'mvhd':b=bytearray(b);b[16:20]=word(112)
  moov+=atom(k,b)
 files['changing.mp4']=ftyp+atom(b'mdat',video+b''.join(packets))+atom(b'moov',moov)
 files['manifest.json']=(json.dumps(dict(cases=cases,provenance='Repository-authored 5.1 tone plus owned C+LFE/single-surround spectra, BSI and neutral VP9. No private media, external encoder, process or network.'),indent=2)+'\n').encode()
 return files
if __name__=='__main__':
 OUT.mkdir(parents=True,exist_ok=True)
 for n,b in generate().items():(OUT/n).write_bytes(b)
