"""Own ALAC signed stereo matrix fixtures; authored PCM and neutral VP9."""
import json,hashlib
from pathlib import Path
from generate_alac_multichannel_fixtures import rice_plane,mp4
from generate_alac_cookie_wrappers import movie
from generate_audio_resample_window_fixture import word
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/alac-stereo-weights'
def values(n):
 sign=-1 if n%2 else 1
 return [(7+(n%8)*3)*sign,-(11+(n%5)*2)*sign]
def packet(depth,extra,shift,weight,start):
 bps=depth-extra+1;planes=[[],[]]
 bits='001'+'0000'+'0'*12+'1'+f'{extra//8:02b}'+'0'+f'{16:032b}'+f'{shift:08b}{weight&255:08b}'+('0000'+'0001'+'100'+'00000')*2
 for n in range(start,start+16):
  left,right=values(n);difference=left-right
  coded=[right+((difference*weight)>>shift),difference]if weight else [left,right]
  for ch,v in enumerate(coded):planes[ch].append(((v+(1<<(bps-1)))&((1<<bps)-1))-(1<<(bps-1)))
  if extra:bits+=f'{(0x12 if extra==8 else 0x1234):0{extra}b}'+f'{(0xab if extra==8 else 0xabcd):0{extra}b}'
 stats=dict(ordinary=0,escapes=0,runs=0,run_samples=0,resumed_run_samples=0)
 for plane in planes:bits+=rice_plane(plane,bps,stats)
 bits+='111';bits+='0'*(-len(bits)%8);return int(bits,2).to_bytes(len(bits)//8,'big')
def generate():
 files={};cases=[]
 for depth,extra in [(16,0),(20,0),(24,8),(24,16),(32,8),(32,16)]:
  cookie=bytearray(24);cookie[:4]=word(16);cookie[5:10]=bytes([depth,40,10,14,2]);cookie[20:24]=word(48000)
  for weight,shift in [(weight,shift)for weight in [-128,-127,-64,-1,0,1,64,127]for shift in [0,1,7,31]]+[(0,255)]:
   packets=[packet(depth,extra,shift,weight,i*16)for i in range(2)];name=f'{depth}-low{extra}-weight{weight}-shift{shift}'
   files[name+'.mkv']=movie(cookie,2,packets);files[name+'.m4a']=mp4(cookie,2,depth,packets);files[name+'.cookie']=bytes(cookie);files[name+'.packet']=packets[0]
   cases.append(dict(name=name,depth=depth,extra=extra,weight=weight,shift=shift))
 cookie=files['16-low0-weight-128-shift0.cookie']
 invalid_packets=[packet(16,0,255,-128,i*16)for i in range(2)]
 files['invalid-negative-shift.mkv']=movie(cookie,2,invalid_packets)
 files['invalid-negative-shift.packet']=invalid_packets[0]
 files['manifest.json']=(json.dumps(dict(provenance='Authored alternating-sign stereo PCM, own weighted matrix/Rice encoder and neutral VP9. Signed8 weight extremes, shifts 0/1/7/31, zero-weight ignored shift255, explicit 8/16 low bits. No private media/external codecs/network.',cases=cases,files={n:hashlib.sha256(v).hexdigest()for n,v in files.items()}),indent=2)+'\n').encode();return files
if __name__=='__main__':
 OUT.mkdir(exist_ok=True)
 for n,b in generate().items():(OUT/n).write_bytes(b)
