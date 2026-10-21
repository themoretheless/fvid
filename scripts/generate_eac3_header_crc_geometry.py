"""Owned CRC-invalid E-AC-3 header geometry in MP4 and Matroska."""
from pathlib import Path
from generate_eac3_aht_lfe import packet,audiovisual
from generate_eac3_bsid import movie
from generate_eac3_playback import crc16_eac3
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/eac3-header-crc-geometry'
def generate():
 good=packet(0,False,None,False);bad=bytearray(good);bad[4]=(bad[4]&0xf1)|(1<<1)
 assert crc16_eac3(good[2:])==0 and crc16_eac3(bad[2:])!=0
 frames=[bytes(bad),good,good]
 return {'control.ec3':good*3,'bad-first.ec3':b''.join(frames),'bad-first.mp4':audiovisual(frames),'bad-first.mkv':movie(frames,channels=3)}
if __name__=='__main__':
 OUT.mkdir(parents=True,exist_ok=True)
 for n,b in generate().items():(OUT/n).write_bytes(b)
