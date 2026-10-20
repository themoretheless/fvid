#!/usr/bin/env python3
"""Authored Layer I bitstreams and independent absolute-convolution PCM.
No encoder, external decoder, private media or network access is used.
"""
from pathlib import Path
import hashlib,json,math,re,struct
from generate_audio_resample_window_fixture import ebml
from generate_vp9_chroma import frame as picture
ROOT=Path(__file__).resolve().parents[1];OUT=ROOT/'tests/fixtures/playback-errors/mp1-native'
window=[int(v)for v in re.findall(r'-?\d+',(ROOT/'crates/fvid-codecs/src/mpeg_synthesis_window.rs').read_text().split('= [',1)[1])]
kernel=[[[window[(lag//2)*64+(lag%2)*32+j]/65536.*math.cos(math.pi*(16+j+(32 if lag%2 else 0))*(2*k+1)/64.)for k in range(32)]for j in range(32)]for lag in range(16)]
class Bits:
 def __init__(self):self.bits=[]
 def put(self,value,n):self.bits.extend((value>>i)&1 for i in range(n-1,-1,-1))
 def pack(self):
  bits=self.bits+[0]*(-len(self.bits)%8)
  return bytes(sum(bits[i+j]<<(7-j)for j in range(8))for i in range(0,len(bits),8))
def crc(bits):
 value=65535
 for bit in bits:
  top=(value>>15)^bit;value=(value<<1)&65535
  if top:value^=0x8005
 return value
def bit_values(data):return [(v>>i)&1 for v in data for i in range(7,-1,-1)]
def convolution(slots,channels):
 sparse=[[[ (k,v)for k,v in enumerate(row[ch])if v]for ch in range(channels)]for row in slots]
 return b''.join(struct.pack('<f',sum(kernel[lag][j][k]*v for lag in range(min(16,t+1))for k,v in sparse[t-lag][ch]))for t in range(len(slots))for j in range(32)for ch in range(channels))
def movie(packets,rate,channels,delay_ns=0,tail_ns=0):
 head=ebml('1a45dfa3',ebml('4282',b'matroska')+ebml('4287',b'\3')+ebml('4285',b'\2'))
 video=ebml('ae',ebml('d7',b'\1')+ebml('73c5',b'\1')+ebml('83',b'\1')+ebml('86',b'V_VP9')+ebml('23e383',(384*1000000000//rate).to_bytes(4,'big'))+ebml('e0',ebml('b0',b'\x10')+ebml('ba',b'\x10')))
 audio=ebml('ae',ebml('d7',b'\2')+ebml('73c5',b'\2')+ebml('83',b'\2')+ebml('86',b'A_MPEG/L1')+(ebml('56aa',delay_ns.to_bytes(8,'big'))if delay_ns else b'')+ebml('e1',ebml('b5',struct.pack('>d',rate))+ebml('9f',bytes([channels]))))
 blocks=ebml('e7',b'\0')
 for i,packet in enumerate(packets):
  stamp=round(i*384*1000/rate)
  blocks+=ebml('a3',b'\x81'+struct.pack('>h',stamp)+b'\x80'+picture(8,'420',16,16))
  audio_block=b'\x82'+struct.pack('>h',stamp)+b'\0'+packet
  blocks+=ebml('a0',ebml('a1',audio_block)+ebml('75a2',tail_ns.to_bytes(8,'big',signed=True)))if tail_ns and i==len(packets)-1 else ebml('a3',audio_block)
 return head+ebml('18538067',ebml('1549a966',ebml('2ad7b1',(1000000).to_bytes(3,'big')))+ebml('1654ae6b',video+audio)+ebml('1f43b675',blocks))
def generate():
 files={};cases=[];clock_cases=[];seed=None;scale_indices=set();bands_seen=set();allocation_codes=set()
 for rate in [48000,44100,32000,24000,22050,16000,12000,11025,8000]:
  version=3 if rate>=32000 else 2 if rate>=16000 else 0
  rate_index=0 if rate in [44100,22050,11025]else 1 if rate in [48000,24000,12000]else 2
  modes=[(m,e,False)for m,e in [(0,0),(2,0),(3,0)]+[(1,e)for e in range(4)]]
  if rate==48000:modes.extend([(0,0,True),(1,1,True),(3,0,True)])
  for mode,extension,dense in modes:
   channels=1 if mode==3 else 2;bound=4*(extension+1)if mode==1 else 32
   for protected in ([True]if dense else[False,True]):
    name=f'{rate}-m{mode}-e{extension}-'+('crc'if protected else'plain')+('-dense'if dense else'');frames=[];free_frames=[];slots=[]
    for number in range(7):
     header=((2047<<21)|(version<<19)|(3<<17)|((not protected)<<16)|(14<<12)|(rate_index<<10)|((number%2)<<9)|(mode<<6)|(extension<<4)).to_bytes(4,'big')
     low=(bound-1-3*number)%bound if mode==1 else number%4
     high=bound+number if mode==1 else 31-number-(2 if mode==2 else 0)
     codes={band:1 for band in range(32)}if dense else{low:2*number+1,high:2*number+2};bands_seen.update(codes);allocation_codes.update(codes.values());w=Bits()
     for band in range(32):
      for ch in range(channels if band<bound else 1):w.put(codes.get(band,0),4)
     allocation_bits=w.bits[:];scales={}
     for band,code in sorted(codes.items()):
      for ch in range(channels):
       sf=30+(band*3+ch+number)%32 if dense else(number*7+band*13+ch*9+mode*5+extension*3+int(protected))%64
       if sf<12 and code<4:sf+=12
       scales[ch,band]=sf;scale_indices.add(sf);w.put(sf,6)
     for sample in range(12):
      row=[[0.]*32 for _ in range(channels)]
      for band,code in sorted(codes.items()):
       width=code+1;levels=1<<width
       for ch in range(channels if band<bound else 1):
        sf=min(scales[target,band]for target in range(channels))if band>=bound else scales[ch,band]
        choices=[0,levels//2-1,levels//2,levels-2,levels-1]if sf>=12 else[levels//2-2,levels//2-1,levels//2]
        raw=choices[(sample+number+ch)%len(choices)];w.put(raw,width)
        # Independent sign-extension + normative C/D coefficients.
        signed=raw-(1<<(width-1));fraction=signed/(1<<(width-1))
        value=(levels/(levels-1))*(fraction+2**(1-width))
        for target in ([ch]if band<bound else range(channels)):
         sf=scales[target,band];row[target][band]=value*(0. if sf==63 else 2**(1-sf/3))
      slots.append(row)
     payload=w.pack();
     if number==0:first_used=4+int(protected)*2+len(payload)
     size=(12000*(448 if version==3 else 256)//rate+number%2)*4
     def packet(h):
      prefix=h+(crc(bit_values(h[2:])+allocation_bits).to_bytes(2,'big')if protected else b'')
      body=prefix+payload;assert len(body)<=size
      return body+bytes(size-len(body))
     frames.append(packet(header));free_head=bytearray(header);free_head[2]&=15;free_frames.append(packet(bytes(free_head)))
    # Header-like ancillary bytes must not establish a false shorter spacing.
    fake=bytearray(free_frames[0]);offset=max((len(fake)//8)*4,((first_used+3)//4)*4)
    assert offset+(6 if protected else 4)<=len(fake)
    fake[offset:offset+(6 if protected else 4)]=fake[:6 if protected else 4]
    assert fake[:first_used]==free_frames[0][:first_used]
    free_frames[0]=bytes(fake)
    pcm=convolution(slots,channels)
    for label,packets in [(name,frames),(name+'-free',free_frames)]:
     files[label+'.mp1']=b''.join(packets);files[label+'.mkv']=movie(packets,rate,channels);files[label+'.f32le']=pcm
     files[label+'.meta']=struct.pack('>IBI7I',rate,channels,len(packets),*map(len,packets))
     cases.append(dict(name=label,rate=rate,channels=channels,mode=mode,extension=extension,protected=protected,dense=dense,free=label.endswith('-free'),sizes=list(map(len,packets)),samples=2688))
    if rate==48000 and mode==3 and protected and not dense:seed=(frames,free_frames)
 for rate in [48000,44100,32000,24000,22050,16000,12000,11025,8000]:
  base=f'{rate}-m1-e2-crc-free';data=files[base+'.mp1'];sizes=struct.unpack('>7I',files[base+'.meta'][9:]);packets=[];at=0
  for size in sizes:packets.append(data[at:at+size]);at+=size
  for label,head,tail,rounded_up in [('delay-tail-floor',17,13,False),('delay-tail-ceil',17,13,True),('tail-ceil',0,13,True),('delay-ceil',17,0,True)]:
   ns=lambda samples:(samples*1000000000+(rate-1 if rounded_up else 0))//rate
   name=f'clock-{rate}-{label}';files[name+'.mkv']=movie(packets,rate,2,ns(head),ns(tail));clock_cases.append(dict(name=name,base=base,rate=rate,channels=2,start=head,end=2688-tail))
 assert scale_indices==set(range(64)),scale_indices
 assert bands_seen==set(range(32)),bands_seen
 assert allocation_codes==set(range(1,15)),allocation_codes
 frames,free=seed
 expected=files['48000-m3-e0-crc-free.f32le']
 files['single-free.mp1']=free[0];files['single-free.mkv']=movie([free[0]],48000,1);files['single-free.f32le']=expected[:384*4]
 small=[packet[:224+(4 if i%2 else 0)]for i,packet in enumerate(free)]
 files['free-small.mp1']=b''.join(small);files['free-small.mkv']=movie(small,48000,1);files['free-small.f32le']=expected
 bad_second=bytearray(small[1]);bad_second[4]^=1
 bad_chain=[small[0],bytes(bad_second)]+small[2:]
 files['bad-second-crc-free.mp1']=b''.join(bad_chain);files['bad-second-crc-free.mkv']=movie(bad_chain,48000,1)

 bad=bytearray(frames[0]);bad[4]^=1
 files['bad-crc.frame']=bytes(bad);files['bad-crc.mkv']=movie([bytes(bad)]+frames[1:],48000,1)
 bad=bytearray(frames[0]);bad[6]|=0xf0;bad[4:6]=crc(bit_values(bad[2:4])+bit_values(bad[6:22])).to_bytes(2,'big')
 files['reserved-allocation.frame']=bytes(bad);files['reserved-allocation.mkv']=movie([bytes(bad)]+frames[1:],48000,1)
 short=free[0][:20];files['truncated.frame']=short;files['truncated.mkv']=movie([short],48000,1)
 changed=free[0]+bytes(4);files['free-spacing.frame']=changed;files['free-spacing.mkv']=movie([free[0],changed]+free[2:],48000,1)
 files['manifest.json']=(json.dumps({'provenance':'Authored Layer I allocations, CRC, quantized symbols and independent signed-fraction/convolution PCM; neutral VP9 video; no encoder/private media/network','spec':'https://ossrs.net/lts/zh-cn/assets/files/ISO_IEC_11172-3-MP3-1993-7e218e3ec360c25149d36152c4e02f08.pdf','cases':cases,'clock_cases':clock_cases,'scalefactors':sorted(scale_indices),'subbands':sorted(bands_seen),'allocation_codes':sorted(allocation_codes),'files':{k:hashlib.sha256(v).hexdigest()for k,v in files.items()}},indent=2)+'\n').encode()
 return files
if __name__=='__main__':
 OUT.mkdir(parents=True,exist_ok=True)
 for name,data in generate().items():(OUT/name).write_bytes(data)
 print('Generated Layer I corpus')
