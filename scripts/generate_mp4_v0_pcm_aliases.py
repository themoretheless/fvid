"""Author classic QT NONE/zero PCM aliases with explicit native controls."""
from pathlib import Path
import json,struct
from generate_mp4_enabled_track_regressions import boxes,canonical_movie,rewrite_track,sample_track
from generate_mp4_seek_io_regressions import generate as video
from generate_audio_resample_window_fixture import atom,word
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/mp4-v0-pcm-aliases'
def generate():
 v=video();base=dict(boxes(v['three-colors.mp4']));ftyp=atom(b'ftyp',b'qt  '+word(0)+b'qt  ');movie=dict(boxes(base[b'moov']));files={f'frame-{i}.yuv16':v[f'frame-{i}.yuv16']for i in range(3)};cases=[];bad=[]
 def wrap(codec,entry,packets,clock,delta):
  audio=sample_track(2,3,b'soun',codec,entry,packets,len(ftyp)+8+len(base[b'mdat']),clock,delta)
  vid=atom(b'trak',rewrite_track(movie[b'trak'],1,3,len(ftyp)+8,False))
  return ftyp+atom(b'mdat',base[b'mdat']+b''.join(packets))+atom(b'moov',atom(b'mvhd',canonical_movie(movie[b'mvhd'],3))+vid+audio)
 for bits in [8,16,24,32]:
  for channels in [1,2]:
   packets=[];pcm=bytearray()
   for p in range(3):
    raw=bytearray()
    for i in range(160):
     for ch in range(channels):
      value=[-0.75,-0.5,-0.125,0.,0.125,0.5][(i+p+ch)%6]
      raw+=int(value*(1<<(bits-1))+(128 if bits==8 else 0)).to_bytes(bits//8,'big',signed=bits!=8);pcm+=struct.pack('<f',value)
    packets.append(bytes(raw))
   stem=f'b{bits}-c{channels}';files[stem+'.f32le']=bytes(pcm)
   entry=bytearray(28);entry[6:8]=b'\0\1';entry[16:20]=struct.pack('>HH',channels,bits);entry[24:28]=word(8000<<16)
   for clock in [8000,1000,1001]:
    control=stem+f'-t{clock}-control.mov';files[control]=wrap(b'raw 'if bits==8 else b'twos',entry,packets,clock,clock//50)
    for codec,name in [(b'NONE','none'),(bytes(4),'zero')]:
     file=stem+f'-t{clock}-'+name+'.mov';files[file]=wrap(codec,entry,packets,clock,clock//50);cases.append(dict(file=file,control=control,expected=stem+'.f32le',bits=bits,channels=channels,rate=8000,frames=480,alias=name))
     if bits==16 and channels==1 and clock==8000:
      for mode,offset,value in [('version',8,1),('compressed',20,1),('unknown-width',18,12)]:
       broken=bytearray(entry);broken[offset:offset+2]=struct.pack('>H',value);unsupported_name='unsupported-'+name+'-'+mode+'.mov';files[unsupported_name]=wrap(codec,broken,packets,clock,clock//50);bad.append(unsupported_name)
 malformed=[]
 for codec,label in [(b'NONE','none'),(bytes(4),'zero')]:
  e=bytearray(28);e[6:8]=b'\0\1';e[16:20]=struct.pack('>HH',1,16);e[24:28]=word(8000<<16);raw=[struct.pack('>160h',*[4096]*160)]*3
  for mode,at,size,value,needle in [('reference',6,2,0,'invalid sample data reference'),('channels',16,2,0,'zero classic QuickTime PCM channel count'),('rate',24,4,0,'zero classic QuickTime PCM sample rate')]:
   broken=bytearray(e);broken[at:at+size]=value.to_bytes(size,'big');name='bad-'+label+'-'+mode+'.mov';files[name]=wrap(codec,broken,raw,8000,160);malformed.append(dict(file=name,needle=needle))
 diagnostics=[dict(c,qualification='Unsupported classic alias width; independent decoder does not match native PCM')for c in cases if c['bits']not in [8,16]]
 cases=[c for c in cases if c['bits']in [8,16]]
 unsupported=bad+[c['file']for c in diagnostics]
 fallback=[]
 for name in bad:
  original=dict(boxes(files[name]));parts=list(boxes(original[b'moov']));good=dict(boxes(files['b16-c1-t8000-control.mov']));audio=next(body for tag,body in boxes(good[b'moov'])if tag==b'trak'and b'soun'in body)
  known=atom(b'trak',rewrite_track(audio,3,3,len(ftyp)+8+len(base[b'mdat']),False));file=name.removesuffix('.mov')+'-then-valid.mov';files[file]=atom(b'ftyp',original[b'ftyp'])+atom(b'mdat',original[b'mdat'])+atom(b'moov',original[b'moov']+known);fallback.append(file)
 files['manifest.json']=(json.dumps(dict(provenance='Authored60ms8/16/24/32bit mono/stereo PCM ramps and distinct VP9 pictures in QuickTime-brand movies. Classic v0 NONE/zero aliases versus native raw/twos controls. No encoder,private media or network.',spec='https://developer.apple.com/documentation/quicktime-file-format/sound_sample_description_version_0',cases=cases,unsupported=unsupported,fallback=fallback,diagnostics=diagnostics,malformed=malformed),indent=2)+'\n').encode();return files
if __name__=='__main__':
 OUT.mkdir(parents=True,exist_ok=True)
 for n,b in generate().items():(OUT/n).write_bytes(b)
