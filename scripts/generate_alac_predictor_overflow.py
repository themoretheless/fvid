"""Authored ALAC predictor rounding overflow, own encoder/containers, offline."""
import hashlib,json,struct
from pathlib import Path
from generate_alac_multichannel_fixtures import rice_plane,mp4,u
from generate_audio_resample_window_fixture import ebml,word
from generate_vp9_chroma import frame
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/alac-predictor-overflow'
def signed(v,bits):return ((v+(1<<(bits-1)))&((1<<bits)-1))-(1<<(bits-1))
def encoded(target):
 coefficient=32767;residual=[target[0],target[1]-target[0]];crossings=0
 for i in range(2,len(target)):
  base=target[i-2];delta=target[i-1]-base
  product=signed(coefficient*delta,32)
  rounded=signed(product+(1<<14),32)>>15
  crossings+=not(-(1<<31)<=product+(1<<14)<(1<<31))
  error=signed(target[i]-base-rounded,25);residual.append(error)
  direction=((base-target[i-1]>0)-(base-target[i-1]<0))*((error>0)-(error<0))
  if error:coefficient=signed(coefficient-direction,16)
 bits='001'+'0000'+'0'*12+'1'+'00'+'0'+f'{len(target):032b}'+'00000001'+'00000001'
 # U has an extreme order-1 predictor, V is identically zero.
 bits+='0000'+'1111'+'100'+'00001'+f'{32767:016b}'
 bits+='0000'+'0001'+'100'+'00000'
 stats=dict(ordinary=0,escapes=0,runs=0,run_samples=0,resumed_run_samples=0)
 bits+=rice_plane(residual,25,stats)+rice_plane([0]*len(target),25,stats)+'111'
 bits+='0'*(-len(bits)%8)
 return int(bits,2).to_bytes(len(bits)//8,'big'),crossings

def generate():
 target=[0,65538]+[65540+(i-2)*2 for i in range(2,16)]
 packet,crossings=encoded(target);assert crossings>0
 cookie=bytearray(24);cookie[:4]=word(16);cookie[5:10]=bytes([24,40,10,14,2]);cookie[20:24]=word(48000)
 video=ebml('ae',u('d7',1)+u('73c5',1)+u('83',1)+ebml('86',b'V_VP9')+ebml('e0',u('b0',16)+u('ba',16)))
 audio=ebml('ae',u('d7',2)+u('73c5',2)+u('83',2)+ebml('86',b'A_ALAC')+ebml('63a2',cookie)+ebml('e1',ebml('b5',struct.pack('>d',48000))+u('9f',2)))
 blocks=u('e7',0)
 for i in range(2):blocks+=ebml('a3',b'\x81'+struct.pack('>hB',i*1000,128)+frame(8,'420',16,16))+ebml('a3',b'\x82'+struct.pack('>hB',i*333,128)+packet)
 movie=ebml('1a45dfa3',ebml('4282',b'matroska')+u('4287',3)+u('4285',2))+ebml('18538067',ebml('1549a966',u('2ad7b1',1000))+ebml('1654ae6b',video+audio)+ebml('1f43b675',blocks))
 files={'rounding.mkv':movie,'rounding.m4a':mp4(cookie,2,24,[packet,packet]),'cookie.bin':bytes(cookie),'packet.bin':packet}
 files['manifest.json']=(json.dumps(dict(provenance='Authored neutral VP9 and 24-bit stereo ALAC; quant=15 coefficient=32767, first product 2147483646 crosses signed32 rounding boundary. Own inversion of literal target PCM. No private media or foreign codecs.',target=target,crossings=crossings,files={n:hashlib.sha256(v).hexdigest()for n,v in files.items()}),indent=2)+'\n').encode()
 return files
if __name__=='__main__':
 OUT.mkdir(exist_ok=True)
 for name,data in generate().items():(OUT/name).write_bytes(data)
