#!/usr/bin/env python3
"""Hand-authored synthetic AVC slices; FFmpeg only muxes and saves oracle YUV."""
from pathlib import Path
import subprocess
import sys
temporal = "--temporal-direct" in sys.argv

class Bits:
    def __init__(self): self.data = []
    def bits(self, value, width): self.data.extend((value >> i) & 1 for i in range(width-1,-1,-1))
    def ue(self, value):
        value += 1
        width = value.bit_length()
        self.bits(0,width-1); self.bits(value,width)
    def se(self, value): self.ue(2*abs(value)-(value>0))
    def align(self):
        while len(self.data)%8: self.bits(0,1)
    def finish(self):
        self.bits(1,1); self.align()
        return bytes(sum(self.data[i+j] << (7-j) for j in range(8)) for i in range(0,len(self.data),8))
    def pcm(self, value):
        self.align()
        for byte in bytes([value])*256 + bytes([128])*128: self.bits(byte,8)

def nal(kind, payload):
    out=bytearray(); zeros=0
    for byte in payload:
        if zeros>=2 and byte<=3: out.append(3); zeros=0
        out.append(byte); zeros=zeros+1 if byte==0 else 0
    return b'\x00\x00\x00\x01'+bytes([kind])+out

s=Bits()
s.bits(77 if temporal else 66,8);s.bits(0,8);s.bits(30,8)
s.ue(0);s.ue(0);s.ue(0 if temporal else 2)
if temporal: s.ue(0)
s.ue(3 if temporal else 2);s.bits(0,1)
s.ue(1);s.ue(0);s.bits(1,1);s.bits(1,1);s.bits(0,1);s.bits(0,1)
p=Bits()
p.ue(0);p.ue(0);p.bits(0,1);p.bits(0,1);p.ue(0);p.ue(0);p.ue(0)
p.bits(0,1);p.bits(0,2);p.se(0);p.se(0);p.se(0);p.bits(1,1);p.bits(0,1);p.bits(0,1)
stream=nal(0x67,s.finish())+nal(0x68,p.finish())

def header(frame, first=0, idr=False, older=False):
    b=Bits();b.ue(first);b.ue(2 if idr else 0);b.ue(0);b.bits(frame,4)
    if idr:
        b.ue(0)
        if temporal: b.bits(frame*4,4)
        b.bits(0,1);b.bits(0,1)
    else:
        if temporal: b.bits(frame*4,4)
        b.bits(0,1) # default active reference count
        b.bits(older,1)
        if older: b.ue(0);b.ue(1);b.ue(3) # currPicNum 2 -> short-term PicNum 0
        b.bits(0,1) # adaptive marking disabled
    b.se(0);b.ue(1) # QP delta, deblocking disabled
    return b

b=header(0,idr=True)
for _ in range(2): b.ue(25);b.pcm(80)
stream+=nal(0x65,b.finish())
b=header(1)
for _ in range(2): b.ue(0);b.ue(30);b.pcm(180)
stream+=nal(0x41,b.finish())
for first in [0,1]:
    b=header(2,first,older=first==1);b.ue(1)
    stream+=nal(0x41,b.finish())
if temporal:
    for first in [0,1]:
        b=Bits();b.ue(first);b.ue(1);b.ue(0);b.bits(3,4);b.bits(6,4)
        b.bits(0,1) # temporal direct
        b.bits(1,1);b.ue(1);b.ue(0) # two L0 references, one L1
        b.bits(0,1);b.bits(0,1) # no list modifications
        b.se(0);b.ue(1);b.ue(1) # QP, disable filter, skip one MB
        stream+=nal(0x01,b.finish())
root=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors'
name='avc-slice-lists-temporal' if temporal else 'avc-slice-lists'
raw=root/(name+'.h264');raw.write_bytes(stream)
video=root/(name+'.mp4');oracle=root/(name+'.yuv')
subprocess.run(['ffmpeg','-v','error','-framerate','30','-i',str(raw),'-c:v','copy','-an','-y',str(video)],check=True)
subprocess.run(['ffmpeg','-v','error','-i',str(raw if temporal else video),'-fps_mode','passthrough','-pix_fmt','yuv420p','-f','rawvideo','-y',str(oracle)],check=True)

if temporal:
    subprocess.run(['ffmpeg','-v','error','-threads','1','-i',str(raw),'-fps_mode','passthrough','-pix_fmt','yuv420p','-f','rawvideo','-y',str(root/(name+'-single-thread.yuv'))],check=True)

# Optional independent reference decode; this executable is never a runtime dependency.
if temporal and '--jm-decoder' in sys.argv:
    executable = Path(sys.argv[sys.argv.index('--jm-decoder')+1]).resolve()
    subprocess.run([str(executable), '-d', str(executable.parent/'decoder.cfg'),
        '-p', 'InputFile='+str(raw.resolve()), '-p', 'OutputFile='+str((root/(name+'-jm.yuv')).resolve())],
        cwd=executable.parent, check=True)
