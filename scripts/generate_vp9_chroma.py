#!/usr/bin/env python3
"""Original neutral VP9 pictures: no encoder, private input or network."""
import ast
import json
import re
import struct
from generate_audio_resample_window_fixture import atom, word, ebml
from pathlib import Path
ROOT = Path(__file__).resolve().parents[1]
DEST = ROOT / 'tests/fixtures/playback-errors'
def frame(depth, chroma, width=64, height=64, level=0, tile=None, segmentation='0', base_q=0):
    profile = (0 if chroma == '420' else 1) + (2 if depth > 8 else 0)
    bits = '10' + str(profile & 1) + str(profile >> 1)
    if profile == 3: bits += '0'
    bits += '0010'  # coded keyframe, shown, not error resilient
    bits += f'{0x498342:024b}'
    if depth > 8: bits += str(int(depth == 12))
    bits += '0010'  # BT.601, limited range
    if profile & 1: bits += {'444':'000', '422':'100', '440':'010'}[chroma]
    bits += f'{width-1:016b}{height-1:016b}' + '0'  # 64 square, render size unchanged
    bits += '0100'  # no refresh context, frame parallel, context zero
    bits += f'{level:06b}' + '0000'  # loop filter level/sharpness/delta enable
    bits += f'{base_q:08b}' + '000' + segmentation  # frame quantizer; optional segment tools
    bits += '0' + f'{32:016b}'  # one tile row, 32-byte compressed header
    bits += '0' * (-len(bits) % 8)
    # Zero arithmetic code selects no probability updates, unsplit blocks,
    # DC prediction and immediate EOB for each transform. Padding is zero.
    return bytes(int(bits[i:i+8], 2) for i in range(0,len(bits),8)) + (bytes(32) + (bytes(4096) if tile is None else tile))
def table(name):
    text=(ROOT/'src/codec/vp9_tables.rs').read_text()
    value=re.search(r'pub const '+name+r':[^=]+=\s*(\[.*?\]);',text,re.S).group(1)
    return ast.literal_eval(value)
class BooleanWriter:
    """Exact integer interval coding, with an arbitrary-precision lower bound."""
    def __init__(self):
        self.low=0; self.range=255; self.shifts=0
        self.write(128,0)  # arithmetic marker
    def write(self,p,bit):
        split=1+((self.range-1)*p >> 8)
        if bit: self.low+=split; self.range-=split
        else: self.range=split
        while self.range < 128:
            self.range <<= 1; self.low <<= 1; self.shifts+=1
    def finish(self):
        bits=f'{self.low:0{8+self.shifts}b}'
        bits+='0'*(-len(bits)%8)
        return bytes(int(bits[i:i+8],2) for i in range(0,len(bits),8))+bytes(8)
def coloured(depth,chroma,side=64,segment=None,dc_quant=4,level=0,base_q=0):
    subs={'420':(1,1),'422':(1,0),'444':(0,0),'440':(0,1)}[chroma]
    coef=table('DEFAULT_COEF_PROBS')[0]; pareto=table('PARETO_TABLE')
    b=BooleanWriter()
    b.write(table('KF_PARTITION_PROBS')[{64:12,16:4}[side]][0],0)
    if segment is not None:
        for p in [91,137,73]: b.write(p,0)  # segment zero path 0/0/0
    b.write(table('DEFAULT_SKIP_PROB')[0],0)
    b.write(table('KF_Y_MODE_PROBS')[0][0][0],0)
    b.write(table('KF_UV_MODE_PROBS')[0][0],0)
    expected=bytearray()
    for plane in range(3):
        sx,sy=subs if plane else (0,0); w,h=side>>sx,side>>sy
        pixels=[0]*(w*h)
        for y in range(0,h,4):
            for x in range(0,w,4):
                ctx=int(plane>0 and x>0)+int(plane>0 and y>0)
                probs=coef[int(plane>0)][0][0][ctx]
                b.write(probs[0],int(plane>0))
                if plane:
                    # Token FOUR: nodes 0,2,4,6,8 followed by sign and EOB.
                    b.write(probs[1],1); b.write(probs[2],1)
                    q=probs[2]; i=(q-1)//2
                    for node,bit in [(2,0),(3,1),(4,1)]:
                        prob=pareto[i][node-2] if q&1 else (pareto[i][node-2]+pareto[i+1][node-2])//2
                        b.write(prob,bit)
                    b.write(128,int(plane==2))
                    b.write(coef[1][0][1][3][0],0)
                neighbours=[]
                if y: neighbours+=pixels[(y-1)*w+x:(y-1)*w+x+4]
                if x: neighbours += [pixels[(y+i)*w+x-1] for i in range(4)]
                dc=(sum(neighbours)+len(neighbours)//2)//len(neighbours) if neighbours else 1<<(depth-1)
                # A lone signed DC coefficient FOUR dequantizes to +/-4*dc_quant.
                # The two separable WHT stages split each DC value into ceil/floor
                # halves (arithmetic shift), so nonmultiples of four give a 4x4
                # pattern. This analytic oracle does not invoke a codec transform.
                q=dc_quant if plane==1 else -dc_quant if plane==2 else 0
                row=[q-q//2,q//2,q//2,q//2]
                for yy in range(4):
                    for xx in range(4):
                        delta=row[xx]-row[xx]//2 if yy==0 else row[xx]//2
                        if base_q:
                            # A lone DCT DC coefficient produces a constant block.
                            # Standard cos(pi/4) fixed-point scaling, twice, followed
                            # by the normative 4x4 output shift; signed rounding is
                            # addition followed by arithmetic shift.
                            dc1=(4*q*11585+8192)>>14
                            dc2=(dc1*11585+8192)>>14
                            delta=(dc2+8)>>4
                        pixels[(y+yy)*w+x+xx]=min((1<<depth)-1,max(0,dc+delta))
        for value in pixels: expected+=value.to_bytes(2,'little')
    return frame(depth,chroma,width=side,height=side,tile=b.finish(),level=level,segmentation=segment if segment is not None else '0',base_q=base_q),expected
def webm(packets):
    video=ebml('b0',b'\x40')+ebml('ba',b'\x40')
    track=ebml('d7',b'\1')+ebml('83',b'\1')+ebml('86',b'V_VP9')+ebml('23e383',(333333333).to_bytes(4,'big'))+ebml('e0',video)
    cluster=ebml('e7',b'\0')
    for i,p in enumerate(packets): cluster+=ebml('a3',b'\x81'+(i*333).to_bytes(2,'big')+b'\x80'+p)
    info=ebml('1549a966',ebml('2ad7b1',(1000000).to_bytes(3,'big')))
    return ebml('1a45dfa3',ebml('4282',b'webm'))+ebml('18538067',info+ebml('1654ae6b',ebml('ae',track))+ebml('1f43b675',cluster))
def mp4(packets,depth,chroma,size=(64,64)):
    count=len(packets)
    ftyp=atom(b'ftyp',b'isom'+word(0)+b'isomvp09');mdat=atom(b'mdat',b''.join(packets))
    entry=bytearray(78);entry[6:8]=(1).to_bytes(2,'big');entry[24:28]=struct.pack('>HH',*size);entry[40:42]=(1).to_bytes(2,'big');entry[74:76]=(24).to_bytes(2,'big')
    stsd=atom(b'stsd',word(0,1)+atom(b'vp09',entry+atom(b'vpcC',bytes([1,0,0,0,(0 if chroma=='420' else 1)+(2 if depth>8 else 0),10,(depth<<4)|({'420':0,'422':2,'444':3}[chroma]<<1),6,6,6,0,0]))))
    stbl=atom(b'stbl',stsd+atom(b'stts',word(0,1,count,1))+atom(b'stsz',word(0,0,count,*map(len,packets)))+atom(b'stsc',word(0,1,1,count,1))+atom(b'stco',word(0,1,len(ftyp)+8))+atom(b'stss',word(0,1,1)))
    dinf=atom(b'dinf',atom(b'dref',word(0,1)+atom(b'url ',word(1))))
    tkhd=bytearray(84);tkhd[12:16]=word(1);tkhd[20:24]=word(count);tkhd[40:44]=tkhd[56:60]=word(65536);tkhd[72:76]=word(0x40000000);tkhd[76:84]=word(size[0]<<16,size[1]<<16)
    mdhd=bytearray(24);mdhd[12:20]=word(50,count);mdhd[20:22]=(21956).to_bytes(2,'big')
    mdia=atom(b'mdia',atom(b'mdhd',mdhd)+atom(b'hdlr',word(0,0)+b'vide'+bytes(12))+atom(b'minf',atom(b'vmhd',word(1)+bytes(8))+dinf+stbl))
    mvhd=bytearray(100);mvhd[12:20]=word(50,count)
    return ftyp+mdat+atom(b'moov',atom(b'mvhd',mvhd)+atom(b'trak',atom(b'tkhd',tkhd)+mdia))

def main():
    cases=[]
    for depth in [8,10,12]:
        for chroma in ['420','422','444','440']:
            name=f'vp9-chroma-{chroma}-{depth}-synthetic.vp9'
            (DEST/name).write_bytes(frame(depth,chroma))
            cases.append(dict(file=name,depth=depth,chroma=chroma,size=[64,64],sample=1 << (depth-1),coloured=f'vp9-chroma-{chroma}-{depth}-coloured.vp9',oracle=f'vp9-chroma-{chroma}-{depth}-coloured.yuv16',webm=f'vp9-chroma-{chroma}-{depth}-synthetic.webm',mp4=None if chroma=='440' else f'vp9-chroma-{chroma}-{depth}-synthetic.mp4',webm_sizes=[[64,64],[17,25],[72,65]]))
            data,oracle=coloured(depth,chroma)
            (DEST/f'vp9-chroma-{chroma}-{depth}-coloured.vp9').write_bytes(data)
            (DEST/f'vp9-chroma-{chroma}-{depth}-coloured.yuv16').write_bytes(oracle)
            if chroma != '440':
                (DEST/f'vp9-chroma-{chroma}-{depth}-synthetic.mp4').write_bytes(mp4([data],depth,chroma))
            (DEST/f'vp9-chroma-{chroma}-{depth}-synthetic.webm').write_bytes(webm([data,frame(depth,chroma,17,25,16),frame(depth,chroma,72,65,16)]))
    (DEST/'vp9-chroma.json').write_text(json.dumps(dict(cases=cases,provenance='Original headers, owned integer-interval Boolean writer, DC prediction and independent analytic lossless residual oracles. Neutral pictures equal the bit-depth midpoint; coloured pictures carry opposite signed chroma DC residuals. No source media, external codec or network.'),indent=2)+'\n')
if __name__ == '__main__': main()
