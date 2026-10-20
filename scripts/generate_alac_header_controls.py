"""Authored ALAC reserved-header bit mutations; unchanged synthetic video/PCM."""
import json,hashlib
from pathlib import Path
import generate_alac_multichannel_fixtures as g
from generate_alac_cookie_wrappers import movie
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/alac-header-controls'
def generate():
 files={};controls=[];invalid=[]
 for channels,depth,compressed in [(1,16,False),(2,16,False),(1,32,True),(2,32,True)]:
  name=f'{channels}-{depth}-'+('rice'if compressed else'raw');groups=[channels]
  cookie=bytearray(24);cookie[:4]=g.word(16);cookie[5:10]=bytes([depth,40,10,14,channels]);cookie[20:24]=g.word(48000)
  stats=dict(ordinary=0,escapes=0,runs=0,run_samples=0,resumed_run_samples=0)
  packets=[g.compressed_packet(depth,groups,i*16,channels,'rice',stats)if compressed else g.packet(depth,groups,i*16,channels)for i in range(2)]
  files[name+'.mkv']=movie(cookie,channels,packets);files[name+'.m4a']=g.mp4(cookie,channels,depth,packets);files[name+'.cookie']=bytes(cookie);files[name+'.packet']=packets[0]
  controls.append(dict(name=name,channels=channels,depth=depth,compressed=compressed))
  mutations=[('reserved-first',0,1,'ALAC element has nonzero reserved header bits'),('reserved-last',2,32,'ALAC element has nonzero reserved header bits')]
  mutations.append(('shifted-code3',2,12,'ALAC element uses reserved shifted-byte code 3'))
  for label,byte,mask,error in mutations:
   bad=[]
   for p in packets:
    p=bytearray(p);p[byte]|=mask;bad.append(bytes(p))
   badname=name+'-'+label;files[badname+'.mkv']=movie(cookie,channels,bad);files[badname+'.packet']=bad[0]
   invalid.append(dict(name=badname,control=name,error=error))
 files['manifest.json']=(json.dumps(dict(provenance='Own raw/Rice SCE/CPE packets and neutral VP9. Only the first/last reserved header bit or reserved shifted-byte code is changed; cookie and body stay authored controls. No private media/external codec/network.',controls=controls,invalid=invalid,files={n:hashlib.sha256(v).hexdigest()for n,v in files.items()}),indent=2)+'\n').encode();return files
if __name__=='__main__':
 OUT.mkdir(exist_ok=True)
 for n,b in generate().items():(OUT/n).write_bytes(b)
