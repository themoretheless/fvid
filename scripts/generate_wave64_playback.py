"""Author Sony Wave64 GUID/size/alignment probes from invented RIFF controls."""
from pathlib import Path
import struct,json
from generate_rf64_playback import generate as rf_wave,chunks
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/wave64-playback'
RIFF=bytes.fromhex('726966662e91cf11a5d628db04c10000');TAIL=bytes.fromhex('f3acd3118cd100c04f8edb8a')
def guid(tag):return tag+TAIL
def chunk(tag,data,size=None,identity=None):return (guid(tag)if identity is None else identity)+struct.pack('<Q',len(data)+24 if size is None else size)+data+bytes((-len(data))%8)
def build(fmt,raw,frames,variant='base'):
 fake=bytearray(guid(b'fmt '));fake[-1]^=1
 body=chunk(b'junk',b'own')+chunk(b'fmt ',b'not-format',identity=bytes(fake))+chunk(b'fmt ',fmt)
 if variant!='nofact':body+=chunk(b'fact',struct.pack('<I'if variant=='fact32'else'<Q',frames))
 body+=chunk(b'data',raw)
 fake=bytearray(guid(b'data'));fake[-1]^=1
 body+=chunk(b'data',b'not-audio',identity=bytes(fake))+chunk(b'junk',b'after-data')
 return RIFF+struct.pack('<Q',40+len(body))+guid(b'wave')+body

def generate():
 parent=rf_wave();files={n:parent[n]for n in ['neutral-video.mp4','frame-0.yuv16','frame-1.yuv16','frame-2.yuv16']};cases=[]
 for c in json.loads(parent['manifest.json'])['cases']:
  if c['mode']!='base':continue
  name=c['name'].removesuffix('-base');control=c['control'];data=parent[control];parts=dict(chunks(data));fmt=parts[b'fmt '];raw=parts[b'data'];files[control]=data
  if c['oracle']is not None:files[c['oracle']]=parent[c['oracle']]
  variants=['base']
  if name in ['classic-t1-b16-c2','speaker-six']:variants+=['nofact','fact32']
  for variant in variants:
   key=name+'-'+variant;files[key+'.w64']=build(fmt,raw,c['frames'],variant);case=dict(c);case.update(name=key,variant=variant);cases.append(case)
 fmt=struct.pack('<HHIIHH',1,1,8000,8000,1,8);raw=bytes([0,127,128,255])*1024+b'\x80';good=build(fmt,raw,4097);bad=[]
 def emit(name,data,needle,unsupported=False):files[name+'.w64']=data;bad.append(dict(file=name+'.w64',needle=needle,unsupported=unsupported))
 b=bytearray(good);b[15]^=1;emit('bad-riff-guid',bytes(b),'Wave64')
 b=bytearray(good);b[39]^=1;emit('bad-wave-guid',bytes(b),'Wave64')
 b=bytearray(good);struct.pack_into('<Q',b,16,39);emit('small-file-size',bytes(b),'Wave64')
 b=bytearray(good);struct.pack_into('<Q',b,16,0xffffffffffffffff);emit('huge-file-size',bytes(b),'file limit')
 b=bytearray(good);struct.pack_into('<Q',b,56,23);emit('small-chunk-size',bytes(b),'chunk size')
 b=bytearray(good);struct.pack_into('<Q',b,56,0xffffffffffffffff);emit('overflow-chunk',bytes(b),'extent')
 b=bytearray(good);extra=chunk(b'data',raw);b+=extra;struct.pack_into('<Q',b,16,len(b));emit('multiple-data',bytes(b),'multiple Wave64 data',True)
 b=bytearray(good);different=bytearray(fmt);struct.pack_into('<I',different,4,16000);b+=chunk(b'fmt ',bytes(different));struct.pack_into('<Q',b,16,len(b));emit('conflicting-fmt',bytes(b),'conflicting Wave64 fmt')
 # Complete audio,missing final metadata. The whole-file claim stays unchanged.
 files['truncated-tail.w64']=good[:-len(chunk(b'junk',b'after-data'))];files['prefix.f32le']=b''.join(struct.pack('<f',(x-128)/128)for x in raw)
 for name,size in [('large-declared',0xffffffff),('large64-declared',1<<32)]:
  prefix=chunk(b'fmt ',fmt)+chunk(b'fact',struct.pack('<Q',size))+guid(b'data')+struct.pack('<Q',24+size)
  expected=40+len(prefix)+size+((-size)%8)
  files[name+'.w64']=RIFF+struct.pack('<Q',expected)+guid(b'wave')+prefix+raw
 files['manifest.json']=(json.dumps(dict(provenance='Full GUID Wave64,header-inclusive64-bit sizes,8-byte alignment,GUID-prefix decoys and analytic/invented RIFF audio controls. No encoder,network or private media.',cases=cases,bad=bad),indent=2)+'\n').encode();return files
if __name__=='__main__':
 OUT.mkdir(parents=True,exist_ok=True)
 for n,b in generate().items():(OUT/n).write_bytes(b)
