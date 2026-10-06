#!/usr/bin/env python3
"""Owned CAVLC PCM/P/B streams forcing cross-mode temporal direct from B motion."""
import argparse, hashlib, json, subprocess, tempfile
from pathlib import Path
from avc_fixture_mp4 import mux, annexb

class Writer:
    def __init__(self): self.bits=[]
    def u(self, value, width=1):
        assert 0 <= value < 1 << width
        self.bits.extend((value >> shift) & 1 for shift in range(width-1,-1,-1))
    def ue(self, value):
        code=value+1; width=code.bit_length()
        self.bits.extend([0]*(width-1)); self.u(code,width)
    def se(self, value): self.ue(2*value-1 if value>0 else -2*value)
    def align(self):
        while len(self.bits)%8: self.u(0)
    def nal(self, header):
        self.u(1); self.align()
        raw=bytes(sum(self.bits[i+j] << (7-j) for j in range(8)) for i in range(0,len(self.bits),8))
        escaped=bytearray([header]); zeros=0
        for value in raw:
            if zeros==2 and value<=3: escaped.append(3); zeros=0
            escaped.append(value); zeros=zeros+1 if value==0 else 0
        return bytes(escaped)

def config(depth, explicit):
    b=Writer(); b.u(110 if depth==10 else 77,8); b.u(0,8); b.u(10,8); b.ue(0)
    if depth==10:
        b.ue(1); b.ue(2); b.ue(2); b.u(0); b.u(0)
    b.ue(0); b.ue(0); b.ue(0); b.ue(3); b.u(0); b.ue(0); b.ue(0)
    b.u(0); b.u(1); b.u(1); b.u(0); b.u(0)
    sps=b.nal(0x67)
    b=Writer(); b.ue(0); b.ue(0); b.u(0); b.u(0); b.ue(0); b.ue(0); b.ue(0)
    b.u(0); b.u(1 if explicit else 2,2); b.se(0); b.se(0); b.se(0); b.u(1); b.u(0); b.u(0)
    pps=b.nal(0x68)
    return bytes([1,110 if depth==10 else 77,0,10,255,225])+len(sps).to_bytes(2,'big')+sps+bytes([1])+len(pps).to_bytes(2,'big')+pps

def picture(depth, kind, frame_num, poc, field, reference, explicit):
    b=Writer(); b.ue(0); b.ue(2 if kind=='I' else 0 if kind=='P' else 1); b.ue(0)
    b.u(frame_num,4); b.u(0)
    if kind=='I': b.ue(0)
    b.u(poc,4)
    if kind.startswith('B'): b.u(0) # temporal direct
    if kind!='I':
        b.u(0); b.u(0) # default ref counts; list0 unmodified
        if kind.startswith('B'): b.u(0)
    if explicit and kind.startswith('B'):
        b.ue(1); b.ue(1) # luma/chroma weight denominators
        for weight, offsets in [(3,(1,2,-1)), (1,(-3,-2,3))]:
            b.u(1); b.se(weight); b.se(offsets[0]); b.u(1)
            for offset in offsets[1:]: b.se(weight); b.se(offset)
    if kind=='I': b.u(0); b.u(0)
    elif reference: b.u(0)
    b.se(0); b.ue(1) # QP 26, no deblocking
    for address in range(2):
        if kind!='I': b.ue(0) # no skipped macroblocks
        if address==0: b.u(int(field))
        if kind=='I':
            b.ue(25); b.align() # owned I_PCM source
            for component in range(3):
                size=16 if component==0 else 8
                for y in range(size):
                    for x in range(size):
                        row=address*size+y
                        value=24+(x*3+row*5+component*37)%176
                        if depth==10: value=value*4+(x+row+component)%4
                        b.u(value,depth)
        else:
            b.ue(0 if kind in ['P','Bdirect'] else 1)
            if kind!='Bdirect':
                if field: b.u(0) # expanded ref_idx_l0=1 (opposite parity)
                vector=(8,4) if address==0 or field else (0,0)
                b.se(vector[0]); b.se(vector[1])
            b.ue(0) # inter coded_block_pattern=0
    header=0x65 if kind=='I' else (0x61 if kind=='P' else 0x41 if reference else 0x01)
    return b.nal(header)

def main():
    p=argparse.ArgumentParser(description=__doc__); p.add_argument('--jm-decoder',type=Path,required=True); a=p.parse_args()
    output=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors'; records=[]
    with tempfile.TemporaryDirectory(prefix='fvid-owned-mbaff-direct-') as temporary:
        directory=Path(temporary)
        for depth in [8,10]:
            for source_field, explicit in [(source_field, explicit) for source_field in [False,True] for explicit in [False,True]]:
                target_field=not source_field; configuration=config(depth, explicit)
                nals=[picture(depth,'I',0,0,False,True,explicit), picture(depth,'P',1,8,False,True,explicit),
                      picture(depth,'Bexplicit',2,4,source_field,True,explicit),
                      picture(depth,'Bdirect',3,2,target_field,False,explicit),
                      picture(depth,'Bexplicit',3,6,target_field,False,explicit)]
                frames=[(pts,index==0,len(nal).to_bytes(4,'big')+nal) for index,(pts,nal) in enumerate(zip([0,4,2,1,3],nals))]
                name='avc-mbaff-colocated-'+('field-to-frame' if source_field else 'frame-to-field')+('-explicit' if explicit else '')+('-high10' if depth==10 else '')+'-cavlc'
                coded=directory/(name+'.264'); oracle=directory/(name+'.yuv'); cfg=directory/'decoder.cfg'; cfg.write_text('')
                coded.write_bytes(annexb(configuration,frames))
                subprocess.run([str(a.jm_decoder),'-d',str(cfg),'-p',f'InputFile={coded}','-p',f'OutputFile={oracle}','-p','FileFormat=0','-p','RefFile=nonexistent.yuv'],cwd=directory,check=True)
                pixels=oracle.read_bytes(); assert len(pixels)==5*16*32*3//2*(2 if depth==10 else 1)
                data=mux(configuration,frames,16,32,25)
                (output/(name+'.mp4')).write_bytes(data); (output/(name+'.yuv')).write_bytes(pixels)
                records.append(dict(file=name+'.mp4',sha256=hashlib.sha256(data).hexdigest(),oracle_sha256=hashlib.sha256(pixels).hexdigest()))
    (output/'avc-mbaff-direct-generated.json').write_text(json.dumps(dict(generator='owned PCM/header/CAVLC writer; no external encoder',jm_decoder_sha256=hashlib.sha256(a.jm_decoder.read_bytes()).hexdigest(),fixtures=records),indent=2)+'\n')
if __name__=='__main__': main()
