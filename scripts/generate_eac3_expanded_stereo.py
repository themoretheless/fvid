"""Owned Annex E.3.8.2 stereo witnesses with distinct base/extended DRC."""
import json
from pathlib import Path
from generate_eac3_dependent import packet
from generate_eac3_programme_selection import identify
from generate_eac3_dialogue_normalization import level
from generate_eac3_bsid import movie as matroska
from generate_eac3_player_stereo import audiovisual
from generate_mp4_enabled_track_regressions import boxes
from generate_audio_resample_window_fixture import atom
from generate_eac3_playback import BitWriter
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/eac3-expanded-stereo'
PLANS=[('7.1',8,[(2,0x0200)]),('10',10,[(2,0x0200),(2,0x0010)]),('16',16,[(6,0x0600),(3,0x0018),(3,0x00a0)])]
def generate():
 files={};cases=[]
 for name,width,deps in PLANS:
  programmes=[]
  for programme in range(2):
   own=[];extended=[]
   for f in range(3):
    base,_=packet(7,True,0,0,'mixed',None,False,blocks_override=3,divisor=1<<programme,gains=[0,32,224],compression=128+programme*16,stereo_mix=([1,0,4,7,6]if f==0 else[2,7,6,0,4]if f==1 else None),lfe_mix_code=[0,31,None][f],compact=True)
    base=level(base,25+programme,31);own.append(base);parts=[identify(base,programme)]
    for i,(layout,channel_map)in enumerate(deps):
     dep,_=packet(layout,False,i,0,'mixed',channel_map,True,last=i==len(deps)-1,blocks_override=3,divisor=1,gains=[96]+[None]*2 if i==len(deps)-1 else None,compression=32 if i==len(deps)-1 else None,compact=True)
     parts.append(level(dep,15,31))
    extended.append(b''.join(parts))
   files[f'{name}-p{programme}-independent.ec3']=b''.join(own)
   programmes.append(extended)
  groups=[programmes[0][f]+programmes[1][f]for f in range(3)]
  files[name+'.ec3']=b''.join(groups);files[name+'.mkv']=matroska(groups,blocks=3,channels=width)
  peak=(max(map(len,groups))*8*48000+767999)//768000;assert 0<peak<=8191
  locations={'7.1':2,'10':2|64,'16':1|2|8|32|64|128}[name]
  w=BitWriter().put(peak,13).put(1,3)
  for _ in range(2):w.put(0,2).put(16,5).zeros(5).put(7,3).put(1,1).zeros(3).put(len(deps),4).put(locations,9)
  setup=bytes(sum(w.bits[i+j]<<(7-j)for j in range(8))for i in range(0,len(w.bits),8))
  def configure(data):
   out=b''
   for kind,body in boxes(data):
    if kind in [b'moov',b'trak',b'mdia',b'minf',b'stbl']:body=configure(body)
    elif kind==b'stsd' and body[12:16]==b'ec-3':
     entry=bytearray(body[16:44]);entry[16:18]=(2).to_bytes(2,'big')
     body=body[:8]+atom(b'ec-3',entry+atom(b'dec3',setup))
    out+=atom(kind,body)
   return out
  files[name+'.mp4']=configure(audiovisual(groups,7,True,width))
  cases.append(dict(name=name,channels=width,rate=48000,programmes=[0,1],samples=2304,independent_dynrng=[0,32,224],dependent_dynrng=96,independent_compression=[128,144],dependent_compression=32,independent_dialnorm=[25,26],dependent_dialnorm=15))
 files['manifest.json']=(json.dumps(dict(cases=cases,provenance='Owned EAC3 independent/extension metadata and neutral VP9; no private media, FFmpeg, external process or network.',standard='ATSC A/52:2018 E.3.8.2, E.3.8.5'),indent=2)+'\n').encode()
 return files
if __name__=='__main__':
 OUT.mkdir(parents=True,exist_ok=True)
 for n,b in generate().items():(OUT/n).write_bytes(b)
