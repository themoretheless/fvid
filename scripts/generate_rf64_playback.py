"""Author bounded RF64/ds64 movies and byte-identical RIFF controls,without encoders."""
from pathlib import Path
import json,struct
from generate_rifx_playback import generate as scalar_wave
from generate_wave_speaker_playback import generate as speaker_wave
from generate_pcm_padding_playback import generate as precision_wave
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/rf64-playback'
def chunk(tag,data,size=None):return tag+struct.pack('<I',len(data)if size is None else size)+data+bytes(len(data)%2)
def chunks(data):
 at=12
 while at+8<=len(data):
  tag=data[at:at+4];n=struct.unpack_from('<I',data,at+4)[0];yield tag,data[at+8:at+8+n];at+=8+n+(n&1)
def build(fmt,raw,frames,mode='base',extra_table=None,tail=True):
 finite=mode=='finite';table=[];middle=chunk(b'fmt ',fmt)
 if mode=='table':
  table=[(b'PAD ',2),(b'JUNK',3),(b'JUNK',5)];middle=chunk(b'JUNK',b'pre',0xffffffff)+middle+chunk(b'JUNK',b'other',0xffffffff)+chunk(b'PAD ',b'xx',0xffffffff)
 else:middle=chunk(b'JUNK',b'own')+middle
 if extra_table is not None:table=extra_table
 if mode!='nofact':middle+=chunk(b'fact',struct.pack('<I',frames if finite else 0xffffffff))
 middle+=chunk(b'data',raw,len(raw)if finite else 0xffffffff)
 if tail:middle+=chunk(b'JUNK',b'after-data')
 ds=struct.pack('<QQQI',0,len(raw),frames,len(table))+b''.join(tag+struct.pack('<Q',size)for tag,size in table)
 body=b'WAVE'+chunk(b'ds64',ds)+middle;size=len(body)
 ds=struct.pack('<QQQI',0xffffffffffffffff if finite else size,0xffffffffffffffff if finite else len(raw),0xffffffffffffffff if finite else frames,len(table))+b''.join(tag+struct.pack('<Q',n)for tag,n in table)
 body=b'WAVE'+chunk(b'ds64',ds)+middle
 return b'RF64'+struct.pack('<I',len(body)if finite else 0xffffffff)+body

def generate():
 base=scalar_wave();files={n:base[n]for n in ['neutral-video.mp4','frame-0.yuv16','frame-1.yuv16','frame-2.yuv16']};profiles=[];cases=[];bad=[]
 for c in json.loads(base['manifest.json'])['cases']:
  key='classic-'+c['name'];profiles.append((key,base[c['name']+'.riff.wav'],base[c['name']+'.f32le'],c['frames'],True))
 sp=speaker_wave();pr=precision_wave()
 for key,data,gold in [('speaker-six',sp['s16-surround.wav'],sp['s16-surround.f32le']),('speaker-rear-float',sp['f32-rear.wav'],sp['f32-rear.f32le']),('speaker-discrete-nine',sp['s16-discrete.wav'],sp['s16-discrete.f32le']),('precision-twenty',pr['s24-v20-c1-valid.wav'],pr['s24-v20-c1-valid.f32le'])]:profiles.append((key,data,gold,4099,True))
 authored=OUT.parent/'coded-seek-history';cm=json.loads((authored/'manifest.json').read_text())
 for c in cm['cases']:
  if c['kind']=='wav':profiles.append(('predictive-'+c['name'],(authored/c['file']).read_bytes(),None,c['frames'],False))
 for key,data,gold,n,external in profiles:
  parts=dict(chunks(data));fmt=parts[b'fmt '];raw=parts[b'data'];files[key+'.riff.wav']=data
  if gold is not None:files[key+'.f32le']=gold
  modes=['base','table','finite']if external else['base']
  if key in ['classic-t1-b16-c2','speaker-six']:modes+=['nofact']
  for mode in modes:
   name=key+'-'+mode;files[name+'.rf64.wav']=build(fmt,raw,n,mode);cases.append(dict(name=name,control=key+'.riff.wav',oracle=key+'.f32le'if gold is not None else None,frames=n,rate=struct.unpack_from('<I',fmt,4)[0],channels=struct.unpack_from('<H',fmt,2)[0],data_bytes=len(raw),mode=mode,external=external))
 # Focused malformed metadata;all PCM bytes remain independently authored.
 fmt=struct.pack('<HHIIHH',1,1,8000,8000,1,8);raw=bytes([0,127,128,255])*1024+b'\x80';valid=build(fmt,raw,4097)
 def emit(name,data,needle):files[name+'.wav']=data;bad.append(dict(file=name+'.wav',needle=needle))
 emit('missing-ds64',b'RF64'+struct.pack('<I',0xffffffff)+b'WAVE'+chunk(b'fmt ',fmt)+chunk(b'data',raw),'ds64')
 emit('short-ds64',b'RF64'+struct.pack('<I',0xffffffff)+b'WAVE'+chunk(b'ds64',bytes(27))+chunk(b'fmt ',fmt),'ds64')
 b=bytearray(valid);struct.pack_into('<I',b,44,0xffffffff);emit('truncated-table',bytes(b),'table')
 b=bytearray(valid);b[48:48]=chunk(b'ds64',struct.pack('<QQQI',len(valid)-8+36,len(raw),4097,0));struct.pack_into('<Q',b,20,len(b)-8);emit('duplicate-ds64',bytes(b),'multiple RF64 ds64')
 b=bytearray(valid);struct.pack_into('<Q',b,20,0xffffffffffffffff);emit('overflow-riff',bytes(b),'RF64 RIFF')
 b=bytearray(valid);struct.pack_into('<Q',b,28,0xffffffffffffffff);emit('huge-data',bytes(b),'data limit')
 b=bytearray(valid);struct.pack_into('<I',b,52,0xffffffff);emit('missing-override',bytes(b),'size override')
 b=bytearray(build(fmt,raw,4097,'table'));struct.pack_into('<Q',b,52,0xffffffffffffffff);emit('huge-table-chunk',bytes(b),'file limit')
 # Last non-audio metadata is missing:complete PCM is not a complete RF64 file.
 files['truncated-tail.wav']=valid[:-len(chunk(b'JUNK',b'after-data'))];files['truncated-tail.f32le']=b''.join(struct.pack('<f',(x-128)/128)for x in raw)
 # Explicit large limits allow reading a small available prefix,not giant reserves.
 for name,size in [('large-declared',0xffffffff),('large64-declared',1<<32)]:
  middle=chunk(b'fmt ',fmt)+chunk(b'fact',struct.pack('<I',0xffffffff))+b'data'+struct.pack('<I',0xffffffff)+raw
  expected=4+36+(len(middle)-len(raw))+size+(size&1)
  ds=struct.pack('<QQQI',expected,size,size,0);files[name+'.wav']=b'RF64'+struct.pack('<I',0xffffffff)+b'WAVE'+chunk(b'ds64',ds)+middle
 files['large-prefix.f32le']=files['truncated-tail.f32le']
 files['manifest.json']=(json.dumps(dict(provenance='Authored RIFF/RF64 pairs with ds64 sizes,three marker policies and repeated chunk IDs. Scalar/extensible PCM,IEEE,G711 and existing invented predictive-code probes;no encoder,network or private media.',cases=cases,bad=bad),indent=2)+'\n').encode();return files
if __name__=='__main__':
 OUT.mkdir(parents=True,exist_ok=True)
 for n,b in generate().items():(OUT/n).write_bytes(b)
