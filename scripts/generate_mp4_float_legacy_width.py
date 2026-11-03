"""Authored v1 QuickTime floats with legacy16 sample-size fields."""
from pathlib import Path
import struct,json
from generate_mp4_enabled_track_regressions import boxes,canonical_movie,rewrite_track,sample_track
from generate_mp4_seek_io_regressions import generate as video
from generate_mp4_variable_rate_edits import edit_movie
from generate_audio_resample_window_fixture import atom,word
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/mp4-float-legacy-width'
def generate():
 v=video();base=dict(boxes(v['three-colors.mp4']));ftyp=atom(b'ftyp',base[b'ftyp']);movie=dict(boxes(base[b'moov']));files={f'frame-{i}.yuv16':v[f'frame-{i}.yuv16']for i in range(3)};cases=[];bad=[];refusals=[]
 for bits in [32,64]:
  for channels in [1,2]:
   packets=[];pcm=bytearray();precise=bytearray()
   for packet in range(3):
    values=[]
    for i in range(960):
     for ch in range(channels):
      value=[-0.75,-0.5,-0.125,0.,0.125,0.5][(i+packet+ch)%6]
      if bits==64 and i%2:value+=2**-40
      values.append(value);pcm+=struct.pack('<f',value);precise+=struct.pack('<d',value)
    packets.append(values)
   expected=f'f{bits}-c{channels}';files[expected+'.f32le']=bytes(pcm);files[expected+'.f64le']=bytes(precise)
   for big in [False,True]:
    coded=[struct.pack(('>'if big else'<')+str(len(p))+('f'if bits==32 else'd'),*p)for p in packets]
    for clock in [47,101,1000,1001,48000,1000000]:
     for nested in [False,True]:
      suffix=f'f{bits}-c{channels}-'+('be'if big else'le')+f'-t{clock}-'+('wave'if nested else'direct');names={}
      for declared in [16,bits]:
       entry=bytearray(44);entry[6:8]=b'\0\1';entry[8:10]=struct.pack('>H',1);entry[16:20]=struct.pack('>HH',channels,declared);entry[20:22]=struct.pack('>h',0);entry[24:28]=word(48000<<16);entry[28:44]=word(1,bits//8,channels*(bits//8),2)
       enda=atom(b'enda',struct.pack('>H',0 if big else 1));entry+=atom(b'wave',enda)if nested else enda
       audio=sample_track(2,3,b'soun',b'fl32'if bits==32 else b'fl64',entry,coded,len(ftyp)+8+len(base[b'mdat']),clock,max(1,clock//50))
       movie_bytes=atom(b'mvhd',canonical_movie(movie[b'mvhd'],3))+atom(b'trak',rewrite_track(movie[b'trak'],1,3,len(ftyp)+8,False))+audio
       if declared==16 and clock==1000 and not big and not nested:
        broken=list(coded);broken[-1]=broken[-1][:channels*(bits//8)//2]
        broken_audio=sample_track(2,3,b'soun',b'fl32'if bits==32 else b'fl64',entry,broken,len(ftyp)+8+len(base[b'mdat']),clock,20)
        broken_movie=atom(b'mvhd',canonical_movie(movie[b'mvhd'],3))+atom(b'trak',rewrite_track(movie[b'trak'],1,3,len(ftyp)+8,False))+broken_audio
        broken_name=f'bad-f{bits}-c{channels}-partial.mov';files[broken_name]=ftyp+atom(b'mdat',base[b'mdat']+b''.join(broken))+atom(b'moov',broken_movie);bad.append(broken_name)
       name=suffix+f'-stated{declared}.mov';files[name]=ftyp+atom(b'mdat',base[b'mdat']+b''.join(coded))+atom(b'moov',movie_bytes);names[declared]=name
      cases.append(dict(file=names[16],control=names[bits],expected=expected+'.f32le',expected64=expected+'.f64le',bits=bits,channels=channels,rate=48000,clock=clock,frames=2880))
 diagnostics=[dict(c,qualification='Unqualified: FFmpeg interprets duration1 PCM tables differently')for c in cases if c['clock']==47]
 cases=[c for c in cases if c['clock']!=47]
 for c in list(cases):
  if c['clock']!=48000:continue
  for mode,edits in [('repeat',[(7,-1),(35,1),(3,-1),(15,960)]),('tail',[(65,0)])]:
   if mode=='tail':
    name=c['file'].removesuffix('.mov')+'-tail-edit.mov';control=c['control'].removesuffix('.mov')+'-tail-edit.mov';files[name]=edit_movie(files[c['file']],edits);files[control]=edit_movie(files[c['control']],edits);refusals.append(dict(file=name,control=control));continue
   expected=f"f{c['bits']}-c{c['channels']}-"+mode;frames=2880 if mode=='repeat'else 3120
   for scalar in ['f32','f64']:
    raw=files[c['expected'if scalar=='f32'else'expected64']];stride=c['channels']*(4 if scalar=='f32'else 8)
    out=bytes(336*stride)+raw[stride:1681*stride]+bytes(144*stride)+raw[960*stride:1680*stride]if mode=='repeat'else raw+bytes(240*stride)
    assert len(out)==frames*stride;files[expected+'.'+scalar+'le']=out
   name=c['file'].removesuffix('.mov')+'-'+mode+'-edit.mov';control=c['control'].removesuffix('.mov')+'-'+mode+'-edit.mov';files[name]=edit_movie(files[c['file']],edits);files[control]=edit_movie(files[c['control']],edits)
   cases.append(dict(c,file=name,control=control,expected=expected+'.f32le',expected64=expected+'.f64le',frames=frames))
 files['manifest.json']=(json.dumps(dict(provenance='Authored60ms float32/float64 ramps with binary-exact samples and low float64 bits, v1 sample_size16 versus native-width controls, direct/nested enda,five qualified clocks plus a separately unqualified duration1-clock diagnostic,repeat edits,explicit past-source refusals and neutral colored VP9. No encoder,private media or network.',cases=cases,bad=bad,refusals=refusals,diagnostics=diagnostics),indent=2)+'\n').encode();return files
if __name__=='__main__':
 OUT.mkdir(parents=True,exist_ok=True)
 for n,b in generate().items():(OUT/n).write_bytes(b)
