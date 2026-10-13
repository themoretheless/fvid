#!/usr/bin/env python3
"""Independent small RFC 9043 RGB v1 traces; authored pixels, offline only."""
import hashlib,json,re
from pathlib import Path
from generate_ffv1_gray_fixtures import element,uint
ROOT=Path(__file__).resolve().parents[1];D=ROOT/'tests/fixtures/playback-errors'
TABLE=list(map(int,re.findall(r'\d+', (ROOT/'crates/fvid-media/src/owned_ffv1_encoder_impl.rs').read_text().split('const TRANSITION: [u8; 256] = [',1)[1].split('];',1)[0])))
assert len(TABLE)==256
class Range:
    def __init__(self):self.low=0;self.range=65280;self.pending=None;self.carries=0;self.data=bytearray()
    def normalize(self):
        if self.low<=65280 or self.low>=65536:
            carry=int(self.low>=65536)
            if self.pending is not None:self.data.append((self.pending+carry)&255)
            self.data+=bytes([0 if carry else 255])*self.carries;self.carries=0;self.pending=self.low>>8
        else:self.carries+=1
        self.low=(self.low&255)<<8;self.range<<=8
    def bit(self,state,index,value):
        split=(self.range*state[index])>>8
        if value:self.low+=self.range-split;self.range=split;state[index]=TABLE[state[index]]
        else:self.range-=split;state[index]=256-TABLE[256-state[index]]
        if self.range<256:self.normalize()
    def integer(self,state,value,signed=False):
        self.bit(state,0,value==0)
        if not value:return
        n=abs(value);exponent=n.bit_length()-1
        for i in range(exponent+1):self.bit(state,1+min(i,9),i<exponent)
        for i in range(exponent-1,-1,-1):self.bit(state,22+min(i,9),(n>>i)&1)
        if signed:self.bit(state,11+min(exponent,10),value<0)
    def finish(self):
        self.low+=255;self.range=255;self.normalize();self.range=255;self.normalize();return bytes(self.data)
def packet(pixels,depth,chroma=True,shifts=(0,0)):
    r=Range();r.bit([128],0,True);h=[128]*32
    for v in [1,1,1,depth]:r.integer(h,v)
    r.bit(h,0,chroma);r.integer(h,shifts[0]);r.integer(h,shifts[1]);r.bit(h,0,False)
    for _ in range(5):r.integer([128]*32,127)
    planes=[[],[],[]];bias=1<<depth
    for red,green,blue in pixels:
        base=blue if 9<=depth<=15 else green
        cb=(green if 9<=depth<=15 else blue)-base;cr=red-base
        for p,v in zip(planes,[base+((cb+cr)>>2),cb+bias,cr+bias]):p.append(v)
    models=[[128]*32,[128]*32];mask=(1<<(depth+1))-1
    def read(p,x,y):
        if y<0:return 0
        if x<0:return 0 if x<-1 or y==0 else p[(y-1)*4]
        return p[y*4+min(x,3)]
    for y in range(2):
        for component,p in enumerate(planes):
            for x in range(4):
                left=read(p,x-1,y);top=read(p,x,y-1);diagonal=read(p,x-1,y-1)
                predictor=sorted([left,top,left+top-diagonal])[1]
                residual=((p[y*4+x]-predictor+bias)&mask)-bias
                r.integer(models[int(component!=0)],residual,True)
    return r.finish(),planes
def movie(packets,depth):
    header=element(0x1a45dfa3,element(0x4282,b'matroska'))
    colour=element(0x55b0,uint(0x55b1,0)+uint(0x55b9,2)+uint(0x55ba,13)+uint(0x55bb,1))
    track=uint(0xd7,1)+uint(0x83,1)+element(0x86,b'V_FFV1')+uint(0x23e383,20000000)+element(0xe0,uint(0xb0,4)+uint(0xba,2)+colour)
    cluster=uint(0xe7,0)+b''.join(element(0xa3,b'\x81'+(i*20).to_bytes(2,'big')+b'\x80'+p) for i,p in enumerate(packets))
    return header+element(0x18538067,element(0x1549a966,uint(0x2ad7b1,1000000))+element(0x1654ae6b,element(0xae,track))+element(0x1f43b675,cluster))
def main():
    cases=[]
    for depth in [8,10,12,16]:
        maxv=(1<<depth)-1;packets=[];frames=[]
        for index in range(2):
            pixels=[(0,0,0),(maxv,0,0),(0,maxv,0),(0,0,maxv),(maxv,maxv,maxv),(1,2,3),(maxv,1,maxv-1),(123+index,77+index,31+index)]
            p,trace=packet(pixels,depth);name=f'ffv1-rgb-depth{depth}-frame{index}'
            samples=bytes(v for pixel in pixels for v in pixel) if depth==8 else b''.join(v.to_bytes(2,'little') for pixel in pixels for v in pixel)
            (D/(name+'.packet')).write_bytes(p);(D/(name+'.rgb')).write_bytes(samples);packets.append(p)
            frames.append(dict(packet=name+'.packet',reference=name+'.rgb',planes=trace,sha256=hashlib.sha256(p).hexdigest()))
        file=f'ffv1-rgb-depth{depth}.mkv';data=movie(packets,depth);(D/file).write_bytes(data)
        cases.append(dict(depth=depth,file=file,sha256=hashlib.sha256(data).hexdigest(),frames=frames))
    invalid=[]
    pixels=[(0,0,0)]*8
    for name,p,error in [
        ('no-chroma',packet(pixels,8,chroma=False)[0],'invalid FFV1 RGB chroma configuration'),
        ('subsampled',packet(pixels,8,shifts=(1,0))[0],'invalid FFV1 RGB chroma configuration'),
        ('sample-range',packet([(256,0,0)]+pixels[1:],8)[0],'FFV1 RGB reconstruction exceeds sample depth')]:
        file='ffv1-rgb-invalid-'+name+'.mkv';data=movie([p],8);(D/file).write_bytes(data)
        packet_file='ffv1-rgb-invalid-'+name+'.packet';(D/packet_file).write_bytes(p)
        invalid.append(dict(file=file,packet=packet_file,error=error,sha256=hashlib.sha256(data).hexdigest()))
    (D/'ffv1-rgb.json').write_text(json.dumps(dict(cases=cases,invalid=invalid,provenance='Independent Python RFC 9043 v1 range coding, zero context quantizers, line-interleaved RCT with mandatory 9-15 bit RGB exception; authored extreme/channel-distinct pixels. No private media or external codec.'),indent=2)+'\n')
if __name__=='__main__':main()
