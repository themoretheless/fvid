"""Author explicit direct PCM byte order beside an unrelated QuickTime wave box."""
from pathlib import Path
import json,struct
from generate_mp4_enabled_track_regressions import boxes,canonical_movie,rewrite_track,sample_track
from generate_mp4_seek_io_regressions import generate as video
from generate_audio_resample_window_fixture import atom,word
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/mp4-enda-wave-fallback'
def generate():
 v=video();base=dict(boxes(v['three-colors.mp4']));movie=dict(boxes(base[b'moov']));ftyp=atom(b'ftyp',b'qt  '+word(0)+b'qt  ');files={f'frame-{i}.yuv16':v[f'frame-{i}.yuv16']for i in range(3)};cases=[];bad=[];bad_controls=[]
 def wrap(codec,entry,packets,clock):
  audio=sample_track(2,3,b'soun',codec,entry,packets,len(ftyp)+8+len(base[b'mdat']),clock,clock//50)
  metadata=atom(b'mvhd',canonical_movie(movie[b'mvhd'],3))+atom(b'trak',rewrite_track(movie[b'trak'],1,3,len(ftyp)+8,False))+audio
  return ftyp+atom(b'mdat',base[b'mdat']+b''.join(packets))+atom(b'moov',metadata)
 for codec,bits,floating in [(b'in24',24,False),(b'in32',32,False),(b'fl32',32,True),(b'fl64',64,True)]:
  for channels in [1,2]:
   values=[]
   for p in range(3):
    vals=[]
    for i in range(960):
     for ch in range(channels):
      n=(i+p+ch)%6
      value=[-.75,-.5,-.125,0.,.125,.5][n]
      if codec==b'fl64' and i%2:value+=2**-40
      if codec==b'in32' and i%2:value+=2**-31
      vals.append(value)
    values.append(vals)
   stem=codec.decode()+f'-c{channels}';files[stem+'.f32le']=b''.join(struct.pack('<f',x)for p in values for x in p);files[stem+'.f64le']=b''.join(struct.pack('<d',x)for p in values for x in p)
   for little in [False,True]:
    packets=[b''.join(struct.pack(('<'if little else'>')+('f'if bits==32 else'd'),x)if floating else int(x*(1<<(bits-1))).to_bytes(bits//8,'little'if little else'big',signed=True)for x in p)for p in values]
    header=bytearray(44 if floating else 28);header[6:8]=b'\0\1';header[8:10]=struct.pack('>H',int(floating));header[16:20]=struct.pack('>HH',channels,16 if floating else bits);header[24:28]=word(48000<<16)
    if floating:header[28:44]=word(1,bits//8,channels*(bits//8),2)
    enda=atom(b'enda',struct.pack('>H',int(little)));wave=atom(b'wave',atom(b'frma',codec))
    for clock in [1000,48000]:
     suffix=stem+('-le'if little else'-be')+f'-t{clock}';control=suffix+'-control.mov';files[control]=wrap(codec,header+enda,packets,clock)
     for order in ['before','after']:
      name=suffix+'-wave-'+order+'.mov';files[name]=wrap(codec,header+(wave+enda if order=='before'else enda+wave),packets,clock);cases.append(dict(file=name,control=control,expected=stem+'.f32le',expected64=stem+'.f64le',codec=codec.decode(),bits=bits,channels=channels,rate=48000,frames=2880,little=little))
    if channels==1 and little:
     for mode,extension in [('invalid-flag',atom(b'enda',b'\0\2')),('short-flag',atom(b'enda',b'\1')),('duplicate',enda+enda)]:
      name=f'bad-{codec.decode()}-{mode}.mov';files[name]=wrap(codec,header+wave+extension,packets,48000);bad.append(dict(file=name,mode=mode));control='control-'+name;files[control]=wrap(codec,header+extension,packets,48000);bad_controls.append(dict(file=control,mode=mode))
 files['manifest.json']=(json.dumps(dict(provenance='Authored60ms signed integer24/32 and float32/64 ramps with low precision bits, mono/stereo, both endian orders, direct enda versus unrelated wave/frma, two clocks and two child orders, three exact VP9 pictures. No encoder,private media or network.',cases=cases,bad=bad,bad_controls=bad_controls),indent=2)+'\n').encode();return files
if __name__=='__main__':
 OUT.mkdir(parents=True,exist_ok=True)
 for n,b in generate().items():(OUT/n).write_bytes(b)
