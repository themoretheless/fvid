#!/usr/bin/env python3
"""Hand-authored synthetic AVC slices; owned MP4 muxing and independent JM YUV."""
from pathlib import Path
import subprocess
import sys
import argparse
import re
import tempfile
from avc_fixture_mp4 import mux, ints

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("--jm-decoder", type=Path, required=True)
parser.add_argument("--jm-config", type=Path, required=True)
parser.add_argument("--output", type=Path, default=Path(__file__).resolve().parents[1]/"tests/fixtures/playback-errors")
for flag in ["mixed-ib", "mixed-bi", "mixed-ip", "mixed-pi", "mixed-pb", "mixed-bp", "temporal-direct"]:
    parser.add_argument("--"+flag, action="store_true")
args = parser.parse_args()
mixed_ib = "--mixed-ib" in sys.argv or "--mixed-bi" in sys.argv
mixed_intra = "--mixed-ip" in sys.argv or "--mixed-pi" in sys.argv or mixed_ib
mixed_pi = "--mixed-pi" in sys.argv or "--mixed-bi" in sys.argv
mixed_pb = "--mixed-pb" in sys.argv or "--mixed-bp" in sys.argv
mixed_bp = "--mixed-bp" in sys.argv
temporal = "--temporal-direct" in sys.argv or mixed_pb or mixed_intra

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
        if mixed_intra:
            intra = first==(1 if mixed_pi else 0)
            b=Bits();b.ue(first);b.ue(2 if intra else (1 if mixed_ib else 0));b.ue(0);b.bits(3,4);b.bits(12,4)
            if not intra:
                if mixed_ib:
                    b.bits(0,1);b.bits(1,1);b.ue(1);b.ue(0)
                    b.bits(0,1);b.bits(0,1)
                else: b.bits(0,1);b.bits(0,1)
            b.se(0);b.ue(1)
            if intra: b.ue(25);b.pcm(100)
            else: b.ue(1)
            stream+=nal(0x01,b.finish())
            continue
        if mixed_pb and first==(1 if mixed_bp else 0):
            b=Bits();b.ue(first);b.ue(0);b.ue(0);b.bits(3,4);b.bits(6,4)
            b.bits(0,1);b.bits(0,1) # default L0, no modifications
            b.se(0);b.ue(1);b.ue(1)
            stream+=nal(0x01,b.finish())
            continue
        b=Bits();b.ue(first);b.ue(1);b.ue(0);b.bits(3,4);b.bits(6,4)
        b.bits(0,1) # temporal direct
        b.bits(1,1);b.ue(1);b.ue(0) # two L0 references, one L1
        b.bits(0,1);b.bits(0,1) # no list modifications
        b.se(0);b.ue(1);b.ue(1) # QP, disable filter, skip one MB
        stream+=nal(0x01,b.finish())
root=args.output
root.mkdir(parents=True, exist_ok=True)
name=('avc-mixed-bi' if mixed_pi else 'avc-mixed-ib') if mixed_ib else ('avc-mixed-pi' if mixed_pi else 'avc-mixed-ip') if mixed_intra else ('avc-mixed-bp' if mixed_bp else 'avc-mixed-pb') if mixed_pb else ('avc-slice-lists-temporal' if temporal else 'avc-slice-lists')
raw=root/(name+'.h264');raw.write_bytes(stream)
video=root/(name+'.mp4');oracle=root/(name+'.yuv')
# The hand-authored fixture has one SPS/PPS, and each picture starts at MB 0.
units = [nal for nal in re.split(b'\x00\x00\x00?\x01',stream) if nal]
sps = next(n for n in units if n[0]&31 == 7)
pps = next(n for n in units if n[0]&31 == 8)
config = bytes([1])+sps[1:4]+bytes([0xff,0xe1])+len(sps).to_bytes(2,'big')+sps+bytes([1])+len(pps).to_bytes(2,'big')+pps
packets, current = [], bytearray()
for unit in units:
    if unit[0]&31 not in [1,5]: continue
    if unit[1]&0x80 and current:
        packets.append(bytes(current));current=bytearray()
    current += ints(len(unit))+unit
packets.append(bytes(current))
assert len(packets)==(4 if temporal else 3)
# POC 0/4/8/6 reorders the temporal-direct picture; intra mixtures use POC 12.
presentation = [0,1,3,2] if temporal and not mixed_intra else list(range(len(packets)))
frames = [(pts,i == 0,packet) for i,(pts,packet) in enumerate(zip(presentation,packets))]
video.write_bytes(mux(config,frames,32,16))
with tempfile.TemporaryDirectory(prefix='fvid-slice-lists-jm-') as directory:
    decoded=Path(directory)/'decoded.yuv'
    subprocess.run([str(args.jm_decoder.resolve()), '-d', str(args.jm_config.resolve()),
        '-p','InputFile='+str(raw.resolve()),'-p','OutputFile='+str(decoded),
        '-p','FileFormat=0','-p','RefFile=nonexistent.yuv'],cwd=directory,check=True)
    reference=decoded.read_bytes()
    assert len(reference)==len(packets)*32*16*3//2
    # Preserve established JM filenames for temporal/mixed acceptance tests.
    (root/(name+('-jm.yuv' if temporal else '.yuv'))).write_bytes(reference)
