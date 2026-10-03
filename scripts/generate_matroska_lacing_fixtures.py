#!/usr/bin/env python3
"""Four-frame laces of existing FVid-generated FFV1 gray packets; no external codecs."""
import struct
from generate_ffv1_gray_fixtures import element, uint, vint, ROOT

OUT = ROOT/'tests/fixtures/playback-errors'

def signed_lace(value):
    for width in range(1,9):
        bias=(1 << (7*width-1))-1
        encoded=value+bias
        if 0<=encoded<(1 << (7*width)):
            return ((1 << (7*width)) | encoded).to_bytes(width,'big')
    raise ValueError('lace difference overflow')

def video(block, group=False, clock=True, padding=0, delay=0):
    header=element(0x1a45dfa3,element(0x4282,b'matroska'))
    info=element(0x1549a966,uint(0x2ad7b1,1000000)+element(0x4489,struct.pack('>d',166)))
    track=uint(0xd7,1)+uint(0x83,1)+element(0x86,b'V_FFV1')
    if clock:track+=uint(0x23e383,40000000)
    if delay:track+=uint(0x56aa,delay)
    track+=element(0xe0,uint(0xb0,4)+uint(0xba,3))
    if group:
        block=block[:3]+bytes([block[3]&0x7f])+block[4:]
        body=element(0xa1,block)+uint(0x9b,161)
        body+=element(0x75a2,padding.to_bytes(8,'big',signed=True))
        block=element(0xa0,body)
    else:block=element(0xa3,block)
    cluster=element(0x1f43b675,uint(0xe7,7)+block)
    return header+element(0x18538067,info+element(0x1654ae6b,element(0xae,track))+cluster)

def main():
    packets=[(OUT/f'ffv1-gray-8-{i}.packet').read_bytes() for i in [0,1,0,1]]
    for mode in ['xiph','ebml','fixed']:
        frames=packets if mode!='fixed' else [packets[0]]*4
        flags={'xiph':2,'ebml':6,'fixed':4}[mode]
        lace=bytes([3])
        if mode=='xiph':
            for frame in frames[:-1]:lace+=bytes([255])*(len(frame)//255)+bytes([len(frame)%255])
        if mode=='ebml':
            (OUT/'matroska-lace-delay.mkv').write_bytes(video(block,delay=10000000))
            lace+=vint(len(frames[0]))
            for previous,frame in zip(frames,frames[1:-1]):lace+=signed_lace(len(frame)-len(previous))
        block=b'\x81'+struct.pack('>hB',-2,0x80|flags)+lace+b''.join(frames)
        (OUT/f'matroska-lace-{mode}.mkv').write_bytes(video(block))
        if mode=='ebml':
            (OUT/'matroska-lace-delay.mkv').write_bytes(video(block,delay=10000000))
            for padding in [-5000000,5000000]:
                (OUT/f'matroska-lace-group-{padding}.mkv').write_bytes(video(block,True,False,padding))
            (OUT/'matroska-lace-no-clock.mkv').write_bytes(video(block,False,False))
        if mode=='fixed':
            (OUT/'matroska-lace-invalid-fixed.mkv').write_bytes(video(block+b'\0'))
    (OUT/'matroska-lace-invalid-xiph.mkv').write_bytes(video(b'\x81\0\0\x82\x01\xff\xff\x01\0'))
    (OUT/'matroska-lace-invalid-ebml.mkv').write_bytes(video(b'\x81\0\0\x86\x02\x81'+signed_lace(-2)+b'\0\0'))

if __name__=='__main__':main()
