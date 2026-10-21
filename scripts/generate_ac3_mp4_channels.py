"""Owned MP4 channelcount-placeholder witnesses from an existing synthetic tone."""
import json,struct,hashlib
from pathlib import Path
from generate_eac3_aht_lfe import audiovisual
from generate_eac3_bsid import movie
from generate_mp4_enabled_track_regressions import boxes
from generate_audio_resample_window_fixture import atom
ROOT=Path(__file__).resolve().parents[1]
OUT=ROOT/'tests/fixtures/playback-errors/ac3-mp4-channels'
def generate():
 source=(ROOT/'tests/fixtures/audio/ac3-51.ac3').read_bytes()
 assert source[:2]==b'\x0b\x77' and source[4]>>6==0
 code=(source[4]&63)//2;size=[32,40,48,56,64,80,96,112,128,160,192,224,256,320,384,448,512,576,640][code]*4
 frames=[source[i*size:(i+1)*size]for i in range(3)]
 assert all(len(p)==size and p[:2]==b'\x0b\x77'for p in frames)
 bits=''.join(f'{v:08b}'for v in source[:10]);acmod=int(bits[48:51],2);assert acmod==7
 at=51+2*int(acmod&1!=0 and acmod!=1)+2*int(acmod&4!=0)+2*int(acmod==2)
 lfe=int(bits[at]);assert lfe==1
 config=f'{0:02b}{source[5]>>3:05b}{source[5]&7:03b}{acmod:03b}{lfe:01b}{code:05b}'+'0'*5
 dac3=int(config,2).to_bytes(3,'big')
 def rewrite(data,channels):
  out=b''
  for kind,body in boxes(data):
   if kind in [b'moov',b'trak',b'mdia',b'minf',b'stbl']:body=rewrite(body,channels)
   elif kind==b'stsd' and body[12:16]==b'ec-3':
    entry=bytearray(body[16:44]);entry[16:18]=struct.pack('>H',channels)
    body=body[:8]+atom(b'ac-3',entry+atom(b'dac3',dac3))
   out+=atom(kind,body)
  return out
 files={'control.ac3':b''.join(frames),'placeholder.mp4':rewrite(audiovisual(frames),2),'native-six.mp4':rewrite(audiovisual(frames),6)}
 files['metadata-two.mkv']=movie(frames,channels=2,codec='A_AC3')
 files['native-six.mkv']=movie(frames,channels=6,codec='A_AC3')
 # Corrupt only acmod in the first frame, deliberately preserving bad CRC.
 # Container six-channel metadata remains correct for the clean following frames.
 damaged=bytearray(frames[0]);damaged[6]=(damaged[6]&31)|(1<<5)
 from generate_eac3_playback import crc16_eac3
 assert crc16_eac3(frames[0][2:])==0 and crc16_eac3(damaged[2:])!=0
 bad_frames=[bytes(damaged)]+frames[1:]
 files['bad-first-header.ac3']=b''.join(bad_frames)
 files['bad-first-header.mp4']=rewrite(audiovisual(bad_frames),6)
 files['bad-first-header.mkv']=movie(bad_frames,channels=6,codec='A_AC3')
 files['manifest.json']=(json.dumps(dict(provenance='Existing repository-authored synthetic 5.1 tone, first three valid 48k AC3 frames, owned neutral VP9 video and MP4/dac3 metadata. No private media, process or network.',source_sha256=hashlib.sha256(source).hexdigest(),frames=3,samples=4608,rate=48000,channels=6),indent=2)+'\n').encode()
 return files
if __name__=='__main__':
 OUT.mkdir(parents=True,exist_ok=True)
 for n,b in generate().items():(OUT/n).write_bytes(b)
