"""Owned changing active speaker sets with a negotiated stable 10-role output."""
import json
from pathlib import Path
from generate_eac3_dependent import packet
from generate_eac3_bsid import movie
from generate_eac3_player_stereo import audiovisual
from generate_eac3_playback import BitWriter
from generate_mp4_enabled_track_regressions import boxes
from generate_audio_resample_window_fixture import atom
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/eac3-output-layout'
def generate():
 files={};cases=[]
 for replacement,initial_six in [(False,False),(True,False),(True,True)]:
  for transient in [False,True]:
   name=('six-'if initial_six else'')+('replace-'if replacement else'')+('transient'if transient else'plain');groups=[];controls=[[],[],[]]if replacement else[[],[]]
   for f in range(3):
    base,_=packet(7,True,0,0,'mixed',None,False,blocks_override=3,divisor=16,compact=True)
    parts=[base];controls[0].append(base)
    for component in range(1,len(controls)):
     windows=[(0,255),(240,127)]if transient else None
     mapping=((0xa000 if initial_six and f!=1 else 0x0200) if component==1 else (0x0010 if f==1 else 0xa000))if replacement else(0x0010 if f==1 else 0x0200)
     dep,_=packet(2,False,component-1,0,'mixed',mapping,True,last=component==len(controls)-1,blocks_override=3,divisor=16<<component,transient_windows=windows,compact=True)
     control,_=packet(2,False,0,0,'mixed',None,False,blocks_override=3,divisor=16<<component,transient_windows=windows,compact=True)
     parts.append(dep);controls[component].append(control)
    groups.append(b''.join(parts))
   files[name+'.ec3']=b''.join(groups);files[name+'.mkv']=movie(groups,blocks=3,channels=6 if initial_six else 8)
   if name=='plain':
    late=[groups[0]]*160+groups[1:]
    files['late.mkv']=movie(late,blocks=3,channels=8)
    for i,c in enumerate(controls):files[f'late-control{i}.ec3']=b''.join([c[0]]*160+c[1:])
   for i,c in enumerate(controls):files[f'{name}-control{i}.ec3']=b''.join(c)
   bitrate=(max(map(len,groups))*8*48000+767999)//768000
   w=BitWriter().put(bitrate,13).put(0,3).put(0,2).put(16,5).zeros(5).put(7,3).put(1,1).zeros(3).put(len(controls)-1,4).put(2|64,9)
   config=bytes(sum(w.bits[i+j]<<(7-j)for j in range(8))for i in range(0,len(w.bits),8))
   def configure(data):
    result=b''
    for k,b in boxes(data):
     if k in [b'moov',b'trak',b'mdia',b'minf',b'stbl']:b=configure(b)
     elif k==b'stsd'and b[12:16]==b'ec-3':
      entry=bytearray(b[16:44]);entry[16:18]=(2).to_bytes(2,'big');b=b[:8]+atom(b'ec-3',entry+(atom(b'dec3',config)if config is not None else b''))
     result+=atom(k,b)
    return result
   files[name+'.mp4']=configure(audiovisual(groups,7,True,6 if initial_six else 8))
   cases.append(dict(name=name,rate=48000,blocks=3,transient=transient,replacement=replacement,active_channels=[6,10,6]if initial_six else[8,10,8]if replacement else[8,8,8],output_channels=10,output_mask=0x563f,samples=2304))
 # Native 1..6-channel programmes with two replacements of C or L/R.
 # The last component wins even when no extension channels are added.
 for width,mode,lfe,dep_mode,mapping in [(1,1,False,1,0x4000),(2,1,True,1,0x4000),(3,3,False,2,0xa000),(4,3,True,2,0xa000),(5,7,False,2,0xa000),(6,7,True,2,0xa000)]:
  for variant in (['plain','transient','gain']if width==6 else['plain','transient']):
   transient=variant=='transient';gain=variant=='gain';final_gains=[0x10,0x40,0x80]if gain else None
   name=('native-six'if width==6 else f'native-{width}')+'-'+variant;groups=[];controls=[[],[],[]]
   for f in range(3):
    base,_=packet(mode,lfe,0,0,'mixed',None,False,blocks_override=3,divisor=16,compact=True,gains=[0x30]*3 if gain else None,compression=0x08 if gain else None)
    base_control,_=packet(mode,lfe,0,0,'mixed',None,False,blocks_override=3,divisor=16,compact=True,gains=final_gains,compression=0x12 if gain else None)
    parts=[base];controls[0].append(base_control)
    for component in [1,2]:
     windows=([(0,255),(240,127)][:1 if dep_mode==1 else 2])if transient else None
     dep,_=packet(dep_mode,False,component-1,0,'mixed',mapping,True,last=component==2,blocks_override=3,divisor=16<<component,transient_windows=windows,compact=True,gains=(final_gains if component==2 else [0x20]*3)if gain else None,compression=0x12 if gain and component==2 else None)
     control,_=packet(dep_mode,False,0,0,'mixed',None,False,blocks_override=3,divisor=16<<component,transient_windows=windows,compact=True,gains=final_gains,compression=0x12 if gain else None)
     parts.append(dep);controls[component].append(control)
    groups.append(b''.join(parts))
   files[name+'.ec3']=b''.join(groups);files[name+'.mkv']=movie(groups,blocks=3,channels=width)
   for i,c in enumerate(controls):files[f'{name}-control{i}.ec3']=b''.join(c)
   w=BitWriter().put(200,13).put(0,3).put(0,2).put(16,5).zeros(5).put(mode,3).put(int(lfe),1).zeros(3).put(2,4).put(0,9)
   config=bytes(sum(w.bits[i+j]<<(7-j)for j in range(8))for i in range(0,len(w.bits),8))
   files[name+'.mp4']=configure(audiovisual(groups,mode,lfe,width))
   config=None;files[name+'-no-dec3.mp4']=configure(audiovisual(groups,mode,lfe,width))
   mask={1:4,2:12,3:7,4:15,5:0x607,6:0x60f}[width]
   cases.append(dict(name=name,rate=48000,blocks=3,transient=transient,replacement=True,active_channels=[width]*3,output_channels=width,output_mask=mask,samples=2304,legacy_no_dec3=name+'-no-dec3.mp4'))
 # ETSI E.1.4 includes Lts/Rts, but dec3.chan_loc has no bit for it.
 # Exercise both initial presence and a late change beyond bounded setup probes.
 for late in [False,True]:
  for transient in [False,True]:
   name='top-'+('late-'if late else'')+('transient'if transient else'plain')
   groups=[];controls=[[],[]];count=162 if late else 3
   for f in range(count):
    base,_=packet(7,True,0,0,'mixed',None,False,blocks_override=3,divisor=16,compact=True)
    windows=[(0,255),(240,127)]if transient else None
    mapping=0x0004 if not late or f==160 else 0x0200
    dep,_=packet(2,False,0,0,'mixed',mapping,True,blocks_override=3,divisor=32,transient_windows=windows,compact=True)
    control,_=packet(2,False,0,0,'mixed',None,False,blocks_override=3,divisor=32,transient_windows=windows,compact=True)
    groups.append(base+dep);controls[0].append(base);controls[1].append(control)
   files[name+'.ec3']=b''.join(groups);files[name+'.mkv']=movie(groups,blocks=3,channels=8)
   for i,c in enumerate(controls):files[f'{name}-control{i}.ec3']=b''.join(c)
   w=BitWriter().put(200,13).put(0,3).put(0,2).put(16,5).zeros(5).put(7,3).put(1,1).zeros(3).put(1,4).put(2 if late else 0,9)
   config=bytes(sum(w.bits[i+j]<<(7-j)for j in range(8))for i in range(0,len(w.bits),8))
   mp4=configure(audiovisual(groups,7,True,8))
   def full_duration(data):
    result=b''
    for k,b in boxes(data):
     if k==b'moov':b=full_duration(b)
     elif k==b'mvhd':b=bytearray(b);b[16:20]=max(60,count*16).to_bytes(4,'big')
     elif k==b'trak'and b'ec-3'in b:
      track=b''
      for kind,body in boxes(b):
       if kind==b'tkhd':body=bytearray(body);body[20:24]=(count*16).to_bytes(4,'big')
       track+=atom(kind,body)
      b=track
     result+=atom(k,b)
    return result
   files[name+'.mp4']=full_duration(mp4)
   cases.append(dict(name=name,rate=48000,blocks=3,transient=transient,late=late,active_channels=[8]*count,output_channels=10 if late else 8,output_mask=None,samples=count*768,chan_loc=2 if late else 0))
 # Re-author only metadata of an existing repository-owned synthetic AAC/PS
 # witness, exercising non-Dolby factory stack size without external encoders.
 from generate_aac_ps_core_clock_fixtures import rewrite_audio
 from generate_he_aac_packet_fixtures import boxes as audio_boxes,box
 original=(OUT.parent/'he-aac-ps-export-repeat-960-synthetic.mp4').read_bytes();rebuilt=[]
 for tag,body in audio_boxes(original):
  if tag==b'moov':body=b''.join(rewrite_audio(k,p)if k==b'trak'and b'mp4a'in p else box(k,p)for k,p in audio_boxes(body))
  rebuilt.append(box(tag,body))
 files['aac-factory-stack.mp4']=b''.join(rebuilt)
 files['manifest.json']=(json.dumps(dict(cases=cases,stack_reproducer='aac-factory-stack.mp4',matroska_automatic_layout='Selected-track header-only full role union, including late.mkv change after 160 programmes.',mp4_automatic_layout='Selected dec3 programme locations negotiate stable output before packet decode.',raw_automatic_layout='Union of selected programme roles; stable 10-channel output without an explicit output-map request; preserve actual per-programme channel counts.',provenance='Owned C/L/R/Ls/Rs/LFE, rear/height/replacement roles and continuous transform/transient controls with neutral VP9; AAC stack witness re-authors metadata of an existing repository-owned synthetic AAC/PS video. No private media, process, external encoder or network.'),indent=2)+'\n').encode()
 return files
if __name__=='__main__':
 OUT.mkdir(parents=True,exist_ok=True)
 for n,b in generate().items():(OUT/n).write_bytes(b)
