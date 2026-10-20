#!/usr/bin/env python3
"""Own Layer II/III audio and neutral VP9 companions for exact seek/clock tests.
Only saved FVid-authored bitstreams/oracles are read. No external process/network.
"""
from pathlib import Path
import hashlib,json,struct
from fractions import Fraction
from generate_audio_resample_window_fixture import ebml
from generate_vp9_chroma import frame as picture
ROOT=Path(__file__).resolve().parents[1];S=ROOT/'tests/fixtures/playback-errors';OUT=S/'mpeg-clock-metadata'
def movie(packets,rate,channels,codec,samples,delay_ns=0,tail_ns=0,gap_ms=0,factor=1.0,shift=False):
 head=ebml('1a45dfa3',ebml('4282',b'matroska')+ebml('4287',b'\3')+ebml('4285',b'\2'))
 video=ebml('ae',ebml('d7',b'\1')+ebml('73c5',b'\1')+ebml('83',b'\1')+ebml('86',b'V_VP9')+ebml('23e383',(samples*1000000000//rate).to_bytes(4,'big'))+ebml('e0',ebml('b0',b'\x10')+ebml('ba',b'\x10')))
 audio=ebml('ae',ebml('d7',b'\2')+ebml('73c5',b'\2')+ebml('83',b'\2')+ebml('86',codec.encode())+(ebml('23314f',struct.pack('>d',factor))if factor!=1.0 else b'')+(ebml('56aa',delay_ns.to_bytes(8,'big'))if delay_ns else b'')+ebml('e1',ebml('b5',struct.pack('>d',rate))+ebml('9f',bytes([channels]))))
 blocks=b'' if shift else ebml('e7',b'\0')
 for i,packet in enumerate(packets):
  stamp=round(i*samples*1000/rate)+(gap_ms if i else 0)
  cluster=i%3 if shift else 0
  if factor!=1.0:
   q=(Fraction(i*samples*1000,rate)-cluster)/Fraction.from_float(factor);stamp=q.numerator//q.denominator
  previous=blocks
  if shift:blocks=ebml('e7',bytes([cluster]))
  blocks+=ebml('a3',b'\x81'+struct.pack('>h',round(i*samples*1000/rate)-cluster if shift else stamp)+b'\x80'+picture(8,'420',16,16))
  block=b'\x82'+struct.pack('>h',stamp)+b'\0'+packet
  blocks+=ebml('a0',ebml('a1',block)+ebml('75a2',tail_ns.to_bytes(8,'big',signed=True)))if tail_ns and i==len(packets)-1 else ebml('a3',block)
  if shift:blocks=previous+ebml('1f43b675',blocks)
 return head+ebml('18538067',ebml('1549a966',ebml('2ad7b1',(1000000).to_bytes(3,'big')))+ebml('1654ae6b',video+audio)+(blocks if shift else ebml('1f43b675',blocks)))
def split(data,sizes):
 out=[];at=0
 for n in sizes:out.append(data[at:at+n]);at+=n
 assert at==len(data);return out
def generate():
 files={};bases=[];cases=[]
 m3={c['rate']:c for c in json.loads((S/'mp3-decoder/manifest.json').read_text())['cases']if c['channels']==2}
 m2free={c['name']:c for c in json.loads((S/'mp2-native/free-manifest.json').read_text())['cases']}
 m2normal={c['name']:c for c in json.loads((S/'mp2-native/manifest.json').read_text())['cases']}
 for layer in [2,3]:
  for rate in [48000,44100,32000,24000,22050,16000,12000,11025,8000]:
   if layer==3:
    case=m3[rate];packets=split((S/'mp3-decoder'/f"{case['name']}.mp3").read_bytes(),case['sizes']);pcm=f"mp3-decoder/{case['name']}.f32le";channels=2;samples=1152 if rate>=32000 else 576
   else:
    sources={48000:'free-0-0-1-48000',44100:'free-1-0-3-44100',32000:'free-1-0-3-32000'}
    if rate in sources:
     name=sources[rate];case=m2free[name];packets=split((S/'mp2-native'/f'{name}.mp2').read_bytes(),case['sizes']);pcm=f'mp2-native/{name}.f32le';channels=case['channels']
    else:
     # Low-frequency Layer II uses the same normative allocation table at all
     # six rates. Re-clock an unprotected authored table-4 stream as free format.
     case=m2normal['4-0-2'];data=(S/'mp2-native/4-0-2.mp2').read_bytes();packets=split(data,[case['frame_bytes']]*2);pcm='mp2-native/4-0-2.f32le';channels=case['channels'];changed=[]
     version=2 if rate>=16000 else 0;index=0 if rate in [22050,11025]else 1 if rate in [24000,12000]else 2
     for packet in packets:
      word=int.from_bytes(packet[:4],'big');assert word&(1<<16)
      word=(word&~((3<<19)|(15<<12)|(3<<10)))|(version<<19)|(index<<10)
      changed.append(word.to_bytes(4,'big')+packet[4:])
     packets=changed
    samples=1152
   name=f'mp{layer}-{rate}';raw=name+f'.mp{layer}';files[raw]=b''.join(packets);files[name+'-control.mkv']=movie(packets,rate,channels,f'A_MPEG/L{layer}',samples)
   if rate==44100:
    for label,factor in [('double',2.0),('third',1/3),('shift-third',1/3)]:
     files[name+'-scaled-'+label+'.mkv']=movie(packets,rate,channels,f'A_MPEG/L{layer}',samples,factor=factor,shift=label.startswith('shift'))
   if rate==48000:
    files[name+'-tick-gap.mkv']=movie(packets,rate,channels,f'A_MPEG/L{layer}',samples,gap_ms=1)
   count=len((S/pcm).read_bytes())//(channels*4);assert count==samples*len(packets)
   base=dict(name=name,raw=raw,rate=rate,channels=channels,layer=layer,source_pcm=pcm,samples=count,frame_samples=samples,packet_count=len(packets));bases.append(base)
   for label,head,tail,up in [('delay-tail-floor',17,13,False),('delay-tail-ceil',17,13,True),('delay-ceil',17,0,True),('tail-ceil',0,13,True)]:
    ns=lambda n:(n*1000000000+(rate-1 if up else 0))//rate
    label=name+'-'+label;files[label+'.mkv']=movie(packets,rate,channels,f'A_MPEG/L{layer}',samples,ns(head),ns(tail));cases.append(dict(base,name=label,start=head,end=count-tail))
 files['manifest.json']=(json.dumps({'provenance':'Own MPEG II/III frame bodies and independent saved PCM oracles; nine-rate table-4 Layer II re-clocking and neutral VP9 companions. No private media, external encoder/decoder, or network.','bases':bases,'cases':cases,'files':{k:hashlib.sha256(v).hexdigest()for k,v in files.items()}},indent=2)+'\n').encode();return files
if __name__=='__main__':
 OUT.mkdir(parents=True,exist_ok=True)
 for name,data in generate().items():(OUT/name).write_bytes(data)
 print('Authored MPEG II/III clock corpus generated')
