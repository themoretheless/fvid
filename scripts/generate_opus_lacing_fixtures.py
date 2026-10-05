#!/usr/bin/env python3
"""Synthetic Opus DTX laces with variable packet durations; no codec executables."""
import struct
from generate_ffv1_gray_fixtures import element,uint,vint,ROOT
from generate_matroska_lacing_fixtures import signed_lace
OUT=ROOT/'tests/fixtures/playback-errors'

def main():
    head=b'OpusHead'+bytes([1,1])+struct.pack('<HIhB',0,48000,0,0)
    packets=[bytes.fromhex(s) for s in ['f8fffe','f9fffefffe','f0fffe','f8fffe']]
    for mode in ['xiph','ebml']:
        lace=bytes([3])
        if mode=='xiph':lace+=bytes(map(len,packets[:-1]))
        else:
            lace+=vint(len(packets[0]))
            for previous,packet in zip(packets,packets[1:-1]):lace+=signed_lace(len(packet)-len(previous))
        for group in [False,True]:
            block=b'\x81\0\0'+bytes([0 if group else 128])+lace+b''.join(packets)
            block=block[:3]+bytes([block[3]|(2 if mode=='xiph' else 6)])+block[4:]
            if group:block=element(0xa0,element(0xa1,block)+uint(0x9b,90))
            else:block=element(0xa3,block)
            track=uint(0xd7,1)+uint(0x83,2)+element(0x86,b'A_OPUS')+element(0x63a2,head)+uint(0x56aa,0)+uint(0x56bb,80000000)
            track+=element(0xe1,element(0xb5,struct.pack('>d',48000))+uint(0x9f,1))
            header=element(0x1a45dfa3,element(0x4282,b'matroska'))
            info=element(0x1549a966,uint(0x2ad7b1,1000000)+element(0x4489,struct.pack('>d',90)))
            data=header+element(0x18538067,info+element(0x1654ae6b,element(0xae,track))+element(0x1f43b675,uint(0xe7,0)+block))
            (OUT/f'opus-lace-{mode}{"-group" if group else ""}.mka').write_bytes(data)
            if mode=='ebml' and group:
                (OUT/'opus-lace-variable-duration-block.mka').write_bytes(data)
                # Named regression for aggregate decoder/index/packet admission.
                clock=0
                blocks=b''
                for packet,duration in zip(packets,[20,40,10,20]):
                    blocks+=element(0xa3,b'\x81'+struct.pack('>h',clock)+b'\x80'+packet)
                    clock+=duration
                memory=header+element(0x18538067,info+element(0x1654ae6b,element(0xae,track))+element(0x1f43b675,uint(0xe7,0)+blocks))
                (OUT/'opus-controlled-memory.mka').write_bytes(memory)
            if mode=='xiph' and not group:
                (OUT/'opus-lace-invalid-packet.mka').write_bytes(data.replace(bytes.fromhex('f8fffe'),bytes.fromhex('fb00fe'),1))
if __name__=='__main__':main()
