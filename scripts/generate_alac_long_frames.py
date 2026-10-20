"""Own long-frame ALAC and neutral VP9; literal periodic signed PCM targets."""
import json,hashlib,struct
from pathlib import Path
from generate_alac_multichannel_fixtures import rice_plane,mp4,u
from generate_audio_resample_window_fixture import ebml,word
from generate_vp9_chroma import frame
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/alac-long-frames'
def values(n):return [101+(n%31)*7,-202-(n%31)*7]
def packet(count,raw,declared=True):
 bits='001'+'0000'+'0'*12+str(int(declared))+'00'+str(int(raw))+(f'{count:032b}'if declared else '')
 if raw:
  bits+=''.join(f'{v&65535:016b}'for n in range(count)for v in values(n))
 else:
  bits+='00000001'+'00000001'+('0000'+'0001'+'100'+'00000')*2
  planes=[[],[]]
  for n in range(count):
   left,right=values(n);planes[0].append(right+((left-right)>>1));planes[1].append(left-right)
  stats=dict(ordinary=0,escapes=0,runs=0,run_samples=0,resumed_run_samples=0)
  for plane in planes:bits+=rice_plane(plane,17,stats)
 bits+='111';bits+='0'*(-len(bits)%8);return int(bits,2).to_bytes(len(bits)//8,'big')
def generate():
 files={};cases=[]
 for maximum,count,raw,declared in [(n,n,raw,declared)for n in [4097,8192,16384,65536]for raw in [False,True]for declared in [False,True]]+[(0xffffffff,16,False,True)]:
  cookie=bytearray(24);cookie[:4]=word(maximum);cookie[5:10]=bytes([16,40,10,14,2]);cookie[20:24]=word(48000)
  payload=packet(count,raw,declared);name=f'{maximum}-'+('raw'if raw else'rice')+(''if declared else'-implicit')
  video=ebml('ae',u('d7',1)+u('73c5',1)+u('83',1)+ebml('86',b'V_VP9')+ebml('e0',u('b0',16)+u('ba',16)))
  audio=ebml('ae',u('d7',2)+u('73c5',2)+u('83',2)+ebml('86',b'A_ALAC')+ebml('63a2',cookie)+ebml('e1',ebml('b5',struct.pack('>d',48000))+u('9f',2)))
  clusters=b''
  for i in range(2):clusters+=ebml('1f43b675',u('e7',i*count*1000000000//48000//1000)+ebml('a3',b'\x81\0\0\x80'+frame(8,'420',16,16))+ebml('a3',b'\x82\0\0\x80'+payload))
  files[name+'.mkv']=ebml('1a45dfa3',ebml('4282',b'matroska')+u('4287',3)+u('4285',2))+ebml('18538067',ebml('1549a966',u('2ad7b1',1000))+ebml('1654ae6b',video+audio)+clusters)
  files[name+'.m4a']=mp4(cookie,2,16,[payload,payload],count)
  cases.append(dict(name=name,maximum=maximum,samples=count,raw=raw,declared=declared))
 # Malformed body: valid maximum-u32 cookie, enormous declared raw count,
 # but no sample payload. This must fail before scratch allocation.
 bits='001'+'0000'+'0'*12+'1'+'00'+'1'+'1'*32+'111'
 bits+='0'*(-len(bits)%8);bad=int(bits,2).to_bytes(len(bits)//8,'big')
 files['maximum.cookie']=bytes(cookie);files['truncated-raw.packet']=bad
 cluster=ebml('1f43b675',u('e7',0)+ebml('a3',b'\x81\0\0\x80'+frame(8,'420',16,16))+ebml('a3',b'\x82\0\0\x80'+bad))
 files['truncated-raw.mkv']=ebml('1a45dfa3',ebml('4282',b'matroska')+u('4287',3)+u('4285',2))+ebml('18538067',ebml('1549a966',u('2ad7b1',1000))+ebml('1654ae6b',video+audio)+cluster)
 files['manifest.json']=(json.dumps(dict(provenance='Authored periodic stereo PCM, own raw/Rice ALAC and neutral VP9. Long frames and maximum-u32 cookie with short explicitly counted packets. No private media or external encoders.',cases=cases,files={n:hashlib.sha256(v).hexdigest()for n,v in files.items()}),indent=2)+'\n').encode();return files
if __name__=='__main__':
 OUT.mkdir(exist_ok=True)
 for n,b in generate().items():(OUT/n).write_bytes(b)
