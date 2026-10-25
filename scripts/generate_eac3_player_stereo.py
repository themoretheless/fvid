"""Owned EAC3 reader/session stereo negotiation witnesses, with neutral VP9."""
import json, struct
from pathlib import Path
from generate_eac3_stereo_downmix import generate as stereo
from generate_eac3_playback import BitWriter
from generate_mp4_enabled_track_regressions import boxes,canonical_movie,rewrite_track,sample_track
from generate_vp9_chroma import frame,mp4
from generate_audio_resample_window_fixture import atom,word
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/eac3-player-stereo'
def audiovisual(packets,layout,lfe,channels):
 base=dict(boxes(mp4([frame(8,'420',16,16)]*3,8,'420',(16,16),sync_samples=[1,2,3])))
 ftyp=atom(b'ftyp',base[b'ftyp']);payload=base[b'mdat'];meta=dict(boxes(base[b'moov']))
 def duration(track,ticks):
  out=b''
  for k,b in boxes(track):
   if k==b'tkhd':b=bytearray(b);b[20:24]=word(ticks)
   out+=atom(k,b)
  return out
 video=atom(b'trak',duration(rewrite_track(meta[b'trak'],1,3,len(ftyp)+8,False),60))
 entry=bytearray(28);entry[6:8]=b'\0\1';entry[16:20]=struct.pack('>HH',channels,16);entry[24:28]=word(48000<<16)
 config=BitWriter().put(2048,13).put(0,3).put(0,2).put(16,5).zeros(5).put(layout,3).put(int(lfe),1).zeros(8)
 assert len(config.bits)==40
 entry+=atom(b'dec3',bytes(sum(config.bits[i+j]<<(7-j)for j in range(8))for i in range(0,40,8)))
 audio=sample_track(2,3,b'soun',b'ec-3',entry,packets,len(ftyp)+8+len(payload),48000,768)
 audio=atom(b'trak',duration(dict(boxes(audio))[b'trak'],48))
 mvhd=canonical_movie(meta[b'mvhd'],3);mvhd[12:20]=word(1000,60)
 return ftyp+atom(b'mdat',payload+b''.join(packets))+atom(b'moov',atom(b'mvhd',mvhd)+video+audio)
def generate():
 names=['l1-f1-c0-s7','l7-f1-c0-s1','l5-f0-c3-s4']
 authored=stereo(names=set(names));meta=json.loads(authored['manifest.json']);files={};cases=[]
 for name in names:
  c=next(c for c in meta['cases']if c['name']==name);cases.append(c)
  files[c['control']]=authored[c['control']]
  for suffix in ['.ec3','.mkv']:files[name+suffix]=authored[name+suffix]
  raw=authored[name+'.ec3'];packets=[raw[i:i+4096]for i in range(0,len(raw),4096)]
  files[name+'.mp4']=audiovisual(packets,c['layout'],c['lfe'],c['channels'])
 files['manifest.json']=(json.dumps(dict(cases=cases,provenance='Owned EAC3 metadata and neutral VP9 authoring. No private source, FFmpeg, process or network.'),indent=2)+'\n').encode()
 return files
if __name__=='__main__':
 OUT.mkdir(parents=True,exist_ok=True)
 for n,b in generate().items():(OUT/n).write_bytes(b)
