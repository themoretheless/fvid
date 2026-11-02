"""Author flat PCM BW64 signals with independent RIFF controls; no encoder."""
from pathlib import Path
import json, struct
from generate_rf64_playback import generate as rf_wave, chunks, chunk
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/bw64-playback'
def build(fmt, raw, mode='base', dummy=0):
 table=[]
 if mode=='table':
  table=[(b'PAD ',2),(b'JUNK',3),(b'JUNK',5)]
  middle=chunk(b'JUNK',b'pre',0xffffffff)+chunk(b'fmt ',fmt)+chunk(b'JUNK',b'other',0xffffffff)+chunk(b'PAD ',b'xx',0xffffffff)
 else:middle=chunk(b'JUNK',b'own')+chunk(b'fmt ',fmt)
 middle+=chunk(b'data',raw,len(raw) if mode=='finite' else 0xffffffff)+chunk(b'JUNK',b'after-data')
 ds=struct.pack('<QQQI',0,len(raw),dummy,len(table))+b''.join(tag+struct.pack('<Q',size)for tag,size in table)
 body=b'WAVE'+chunk(b'ds64',ds)+middle
 ds=struct.pack('<QQQI',0xffffffffffffffff if mode=='finite' else len(body),0xffffffffffffffff if mode=='finite' else len(raw),dummy,len(table))+b''.join(tag+struct.pack('<Q',size)for tag,size in table)
 body=b'WAVE'+chunk(b'ds64',ds)+middle
 return b'BW64'+struct.pack('<I',len(body) if mode=='finite' else 0xffffffff)+body

def generate():
 parent=rf_wave();files={n:parent[n]for n in ['neutral-video.mp4','frame-0.yuv16','frame-1.yuv16','frame-2.yuv16']};cases=[]
 for c in json.loads(parent['manifest.json'])['cases']:
  if c['mode']!='base' or not c['external']:continue
  parts=dict(chunks(parent[c['control']]));fmt=parts[b'fmt '];raw=parts[b'data']
  if struct.unpack_from('<H',fmt)[0] not in [1,3,0xfffe]:continue
  key=c['name'].removesuffix('-base');files[c['control']]=parent[c['control']];files[c['oracle']]=parent[c['oracle']]
  for mode,dummy in [('base',0),('table',0),('finite',0),('dummy',0xffffffffffffffff)]:
   name=key+'-'+mode;files[name+'.bw64.wav']=build(fmt,raw,mode,dummy);case=dict(c);case.update(name=name,mode=mode);cases.append(case)
 fmt=struct.pack('<HHIIHH',1,1,8000,8000,1,8);raw=bytes([0,127,128,255])*1024+b'\x80';valid=build(fmt,raw)
 bad=[]
 for c in json.loads(parent['manifest.json'])['bad']:
  data=parent[c['file']];assert data.startswith(b'RF64');files[c['file']]=b'BW64'+data[4:];bad.append(c)
 files['truncated-tail.wav']=valid[:-len(chunk(b'JUNK',b'after-data'))]
 files['truncated-tail.f32le']=parent['truncated-tail.f32le'];files['large-prefix.f32le']=parent['large-prefix.f32le']
 for name,size in [('large-declared',0xffffffff),('large64-declared',1<<32)]:
  middle=chunk(b'fmt ',fmt)+b'data'+struct.pack('<I',0xffffffff)+raw
  expected=4+36+len(middle)-len(raw)+size+(size&1)
  files[name+'.wav']=b'BW64'+struct.pack('<I',0xffffffff)+b'WAVE'+chunk(b'ds64',struct.pack('<QQQI',expected,size,0,0))+middle
 files['manifest.json']=(json.dumps(dict(provenance='Authored flat integer/float/extensible BW64, no fact. ds64 dummy fields are zero or deliberately nonzero ignored values per ITU-R BS.2088. No ADM rendering claim, private source or encoder.',cases=cases,bad=bad),indent=2)+'\n').encode();return files
if __name__=='__main__':
 OUT.mkdir(parents=True,exist_ok=True)
 for n,b in generate().items():(OUT/n).write_bytes(b)
