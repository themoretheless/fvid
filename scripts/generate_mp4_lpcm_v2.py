"""Author packed/interleaved QuickTime v2 LPCM and legacy controls offline."""
from pathlib import Path
import json,struct
from generate_mp4_enabled_track_regressions import boxes,canonical_movie,rewrite_track,sample_track
from generate_vp9_chroma import frame,mp4
from generate_audio_resample_window_fixture import atom,word
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/mp4-lpcm-v2'
def generate():
 base=dict(boxes(mp4([frame(8,'420',16,16)]*3,8,'420',(16,16),sync_samples=[1,2,3])))
 ftyp=atom(b'ftyp',base[b'ftyp']);movie=dict(boxes(base[b'moov']));files={};cases=[]
 def wrap(codec,entry,packets,rate,frames_per_packet=1):
  video=atom(b'trak',rewrite_track(movie[b'trak'],1,3,len(ftyp)+8,False))
  audio=sample_track(2,3,b'soun',codec,entry,packets,len(ftyp)+8+len(base[b'mdat']),rate,frames_per_packet)
  return ftyp+atom(b'mdat',base[b'mdat']+b''.join(packets))+atom(b'moov',atom(b'mvhd',canonical_movie(movie[b'mvhd'],3))+video+audio)
 for kind,bits in [('u',8),('i',8),('i',16),('i',24),('i',32),('f',32),('f',64)]:
  for big in [False,True]:
   for channels in [1,2]:
    for rate in [48000,96000]:
     name=f'{kind}{bits}-'+('be' if big else 'le')+f'-c{channels}-r{rate}';packets=[];pcm=bytearray();pcm64=bytearray()
     for i in range(12):
      packet=bytearray()
      for channel in range(channels):
       # Binary-exact fractions make the float32 oracle independent of rounding.
       v=[-0.75,-0.5,-0.125,0.,0.125,0.5][(i+channel)%6]
       if kind=='f'and bits==64 and i%2:v+=2**-40
       pcm64+=struct.pack('<d',v)
       if kind=='f':packet+=struct.pack(('>'if big else '<')+('f'if bits==32 else 'd'),v)
       else:packet+=int(v*(1<<(bits-1))+(128 if kind=='u'else 0)).to_bytes(bits//8,'big'if big else'little',signed=kind=='i')
       pcm+=struct.pack('<f',v)
      packets.append(bytes(packet))
     flags=8+(2 if big else 0)+(1 if kind=='f'else 4 if kind=='i'else 0)
     entry=bytearray(64);entry[6:8]=b'\0\1';entry[8:10]=struct.pack('>H',2);entry[16:28]=struct.pack('>HHhHI',3,16,-2,0,65536)
     entry[28:]=struct.pack('>IdIIIIII',72,float(rate),channels,0x7f000000,bits,flags,channels*(bits//8),1)
     files[name+'.mov']=wrap(b'lpcm',entry,packets,rate);files[name+'.f32le']=bytes(pcm);files[name+'.f64le']=bytes(pcm64)
     c=dict(file=name+'.mov',expected=name+'.f32le',expected64=name+'.f64le',rate=rate,channels=channels,frames=12,bits=bits,flags=flags)
     if rate==48000:
      legacy=bytearray(28);legacy[6:8]=b'\0\1';legacy[16:20]=struct.pack('>HH',channels,bits);legacy[24:28]=word(rate<<16)
      codec=b'raw 'if kind=='u'else (b'fl32'if bits==32 else b'fl64')if kind=='f'else b'twos'if big else b'sowt'
      if kind=='f':legacy+=atom(b'enda',struct.pack('>H',0 if big else 1))
      control=name+'-legacy.mov';files[control]=wrap(codec,legacy,packets,rate);c['control']=control
     cases.append(c)
     grouped=bytearray(entry);grouped[56:64]=word(channels*(bits//8)*3,3)
     grouped_name=name+'-grouped.mov';files[grouped_name]=wrap(b'lpcm',grouped,[b''.join(packets[i:i+3])for i in range(0,12,3)],rate,3)
     grouped_case=dict(c,file=grouped_name);grouped_case.pop('control',None);cases.append(grouped_case)
 bad=[];unsupported=[]
 original=files['i16-le-c1-r48000.mov'];at=original.index(b'lpcm')+4
 for name,offset,payload,needle in [
  ('short-structure',28,word(64),'LPCM structure size'),
  ('bad-marker',44,word(0),'LPCM marker'),
  ('infinite-rate',32,struct.pack('>d',float('inf')),'LPCM sample rate'),
  ('zero-channels',40,word(0),'LPCM channel count'),
  ('zero-packet-frames',60,word(0),'LPCM packet geometry'),
  ('wrong-packet-bytes',56,word(3),'LPCM packet geometry')]:
  data=bytearray(original);data[at+offset:at+offset+len(payload)]=payload;file=name+'.mov';files[file]=bytes(data);bad.append(dict(file=file,needle=needle))
 for name,flags in [('planar',44),('unpacked',4),('unknown-flags',0x100c)]:
  data=bytearray(original);data[at+52:at+56]=word(flags);file=name+'.mov';files[file]=bytes(data);unsupported.append(file)
 files['manifest.json']=(json.dumps(dict(provenance='Authored12-frame binary-exact PCM plus three neutral VP9 keyframes; packed interleaved v2 LPCM and legacy48kHz controls. No external encoder, private media or network.',spec='https://developer.apple.com/documentation/quicktime-file-format/sound_sample_description_version_2',cases=cases,bad=bad,unsupported=unsupported),indent=2)+'\n').encode();return files
if __name__=='__main__':
 OUT.mkdir(parents=True,exist_ok=True)
 for n,b in generate().items():(OUT/n).write_bytes(b)
