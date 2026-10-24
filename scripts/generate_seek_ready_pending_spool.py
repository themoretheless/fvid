"""Owned short VP9 movie for a ready seek picture with pending future I/O."""
import struct
from pathlib import Path
from generate_eac3_presentation import element,uint,frame
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/seek-ready-pending-spool.mkv'
def generate():
 track=uint(0xd7,1)+uint(0x73c5,1)+uint(0x83,1)+element(0x86,b'V_VP9')+uint(0x23e383,40000000)+element(0xe0,uint(0xb0,16)+uint(0xba,16))
 clusters=b''.join(element(0x1f43b675,uint(0xe7,n*40)+element(0xa3,b'\x81\0\0\x80'+frame(8,'420',16,16)))for n in range(8))
 return element(0x1a45dfa3,element(0x4282,b'matroska'))+element(0x18538067,element(0x1549a966,uint(0x2ad7b1,1000000)+element(0x4489,struct.pack('>d',320.)))+element(0x1654ae6b,element(0xae,track))+clusters)
if __name__=='__main__':OUT.write_bytes(generate())
