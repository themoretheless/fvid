"""Authored constant selected stsd behind unused supported/opaque entries."""
from pathlib import Path
import json,struct
from generate_mp4_enabled_track_regressions import boxes
from generate_mp4_uniform_plan_scan import generate as compact
from generate_mp4_seek_io_regressions import generate as video
from generate_mp4_audio_reconfiguration import generate as reconfigured
from generate_audio_resample_window_fixture import atom,word
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/mp4-constant-selected-description'
def generate():
 a=compact();v=video();files={'audio-control.mp4':a['frames-64.mp4'],'video-control.mp4':v['three-colors.mp4'],**{f'frame-{i}.yuv16':v[f'frame-{i}.yuv16']for i in range(3)}};cases=[];refusals=[]
 r=reconfigured(pcm_shapes=[(b'sowt',48000,1,16)]*3,pcm_only=True);files['audio-fragmented-control.mp4']=r['pcm-fragmented.mp4'];files['audio-fragmented.f32le']=b''.join(r[f'pcm-{i}.f32le']for i in range(6));files['audio.f32le']=struct.pack('<64f',*[0.125]*64)
 for kind,base in [('audio',files['audio-control.mp4']),('video',files['video-control.mp4']),('audio-fragmented',files['audio-fragmented-control.mp4'])]:
  modes=[('supported-first',['other','actual'],2),('opaque-first',['opaque','actual'],2),('both-first',['other','opaque','actual'],3),('actual-first',['actual','other'],1),('opaque-active',['opaque','actual'],1)]
  if kind.startswith('audio'):modes += [('version-first',['version','actual'],2),('version-active',['version','actual'],1)]
  for mode,order,selected in modes:
   def rewrite(data,active=False):
    out=b''
    for k,b in boxes(data):
     if k==b'trak':b=rewrite(b,(b'soun'if kind.startswith('audio')else b'vide')in b)
     elif k in [b'moov',b'mdia',b'minf',b'stbl',b'mvex',b'moof',b'traf']:b=rewrite(b,active)
     elif active and k==b'stsd':
      codec,body=next(boxes(b[8:]));other=bytearray(body)
      if kind.startswith('audio'):other[16:20]=struct.pack('>HH',2,32);other[24:28]=word(32000<<16);other=atom(b'fl32',other+atom(b'enda',b'\0\1'))
      else:other[24:28]=struct.pack('>HH',32,32);other=atom(codec,other)
      version=bytearray(body);version[8:10]=struct.pack('>H',2)
      entries={'version':atom(codec,version),'actual':atom(codec,body),'other':other,'opaque':atom(b'zzzz',bytes(6)+b'\0\1')}
      b=word(0,len(order))+b''.join(entries[name]for name in order)
     elif active and k==b'stsc':
      b=bytearray(b)
      for n in range(struct.unpack_from('>I',b,4)[0]):b[16+n*12:20+n*12]=word(selected)
     elif k in [b'trex',b'tfhd'] and struct.unpack_from('>I',b,4)[0]==2:
      b=bytearray(b);b[8:12]=word(selected)
     out+=atom(k,b)
    return out
   name=kind+'-'+mode+'.mp4';files[name]=rewrite(base);case=dict(file=name,kind='audio'if kind.startswith('audio')else'video',control=kind+'-control.mp4',selected=selected,frames=5760 if kind=='audio-fragmented'else 64 if kind=='audio'else 3,expected=kind+'.f32le'if kind.startswith('audio')else None)
   (refusals if mode in ['opaque-active','version-active']else cases).append(case)
 files['manifest.json']=(json.dumps(dict(provenance='Authored64-frame constant PCM and three distinct VP9 pictures. Unused supported/opaque stsd entries; whole-run explicit stsc selection. No encoder,private media or network.',cases=cases,refusals=refusals),indent=2)+'\n').encode();return files
if __name__=='__main__':
 OUT.mkdir(parents=True,exist_ok=True)
 for n,b in generate().items():(OUT/n).write_bytes(b)
