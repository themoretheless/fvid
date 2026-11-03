"""Author equal-size dormant PCM/MACE catalogs with unchanged selected PCM."""
from pathlib import Path
import json,struct
from generate_mp4_seek_io_regressions import generate as video
from generate_mp4_enabled_track_regressions import canonical_movie,rewrite_track,sample_track
from generate_mp4_v0_pcm_aliases import generate as originals
from generate_mp4_enabled_track_regressions import boxes
from generate_audio_resample_window_fixture import atom,word
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/mp4-unused-expansion-budget'
def generate():
 base=originals();files={f'frame-{i}.yuv16':base[f'frame-{i}.yuv16']for i in range(3)};cases=[];controls=[]
 for ch in [1,2]:
  source=base[f'b16-c{ch}-t8000-control.mov'];oracle=f'c{ch}.f32le';files[oracle]=base[f'b16-c{ch}.f32le']
  for selected in [1,2]:
   def rewrite(data,tag,active=False):
    out=b''
    for k,b in boxes(data):
     if k==b'trak':b=rewrite(b,tag,b'soun'in b)
     elif k in [b'moov',b'mdia',b'minf',b'stbl']:b=rewrite(b,tag,active)
     elif active and k==b'stsd':
      codec,e=next(boxes(b[8:]));actual=atom(codec,e);dormant=bytearray(e);dormant[18:20]=struct.pack('>H',8 if tag in [b'MAC3',b'MAC6']else 16);dormant=atom(tag,dormant);b=word(0,2)+(actual+dormant if selected==1 else dormant+actual)
     elif active and k==b'stsc':
      b=bytearray(b)
      for n in range(int.from_bytes(b[4:8],'big')):b[16+n*12:20+n*12]=word(selected)
     out+=atom(k,b)
    return out
   control=f'c{ch}-selected{selected}-pcm-catalog.mov';files[control]=rewrite(source,b'twos');controls.append(dict(file=control,oracle=oracle,channels=ch,budget=(48 if ch==1 else 64)*1024))
   for codec in [b'MAC3',b'MAC6']:
    name=f'c{ch}-selected{selected}-unused-{codec.decode()}.mov';files[name]=rewrite(source,codec);cases.append(dict(file=name,control=control,oracle=oracle,channels=ch,budget=(48 if ch==1 else 64)*1024,selected=selected))
 active=[];v=dict(boxes(video()['three-colors.mp4']));ftyp=atom(b'ftyp',v[b'ftyp']);movie=dict(boxes(v[b'moov']))
 for ch in [1,2]:
  for tag,mult,cap in [(b'MAC3',1024,80 if ch==1 else 144),(b'MAC6',512,56 if ch==1 else 80)]:
   size=mult*ch;payload=bytes((i*29+7)%256 for i in range(size));entry=bytearray(28);entry[6:8]=b'\0\1';entry[16:20]=struct.pack('>HH',ch,8);entry[24:28]=word(8000<<16)
   init=sample_track(2,3,b'soun',tag,entry,[],0,8000,0)
   def empty(data):
    out=b''
    for k,b in boxes(data):
     if k in [b'trak',b'mdia',b'minf',b'stbl']:b=empty(b)
     elif k in [b'stts',b'stsc',b'stco']:b=word(0,0)
     elif k==b'stsz':b=word(0,0,0)
     out+=atom(k,b)
    return out
   init=empty(init);mvhd=bytearray(canonical_movie(movie[b'mvhd'],3));mvhd[16:20]=word(58)
   vid=atom(b'trak',rewrite_track(movie[b'trak'],1,3,len(ftyp)+8,False));metadata=atom(b'moov',atom(b'mvhd',mvhd)+vid+init+atom(b'mvex',atom(b'trex',word(0,2,1,3072,size,0))))
   output=ftyp+atom(b'mdat',v[b'mdat'])+metadata
   for i in range(3):
    def moof(offset):return atom(b'moof',atom(b'mfhd',word(0,i+1))+atom(b'traf',atom(b'tfhd',word(0x020018,2,3072,size))+atom(b'tfdt',word(0,3072*i))+atom(b'trun',word(1,1,offset))))
    output+=moof(len(moof(0))+8)+atom(b'mdat',payload)
   name=f'active-{tag.decode()}-c{ch}.mov';files[name]=output;files[name+'.coded']=payload*3;active.append(dict(file=name,coded=name+'.coded',codec=tag.decode(),channels=ch,packet_bytes=size,frames=9216,budget=cap*1024))
 files['manifest.json']=(json.dumps(dict(provenance='Authored60ms VP9 plus signed big-endian16-bit mono/stereo PCM ramps. Equal-size two-entry catalogs with identical PCM or unused MACE; all samples retain one selected PCM header. No private media, encoder or network.',cases=cases,controls=controls,active_mace=active),indent=2)+'\n').encode();return files
if __name__=='__main__':
 OUT.mkdir(parents=True,exist_ok=True)
 for n,b in generate().items():(OUT/n).write_bytes(b)
