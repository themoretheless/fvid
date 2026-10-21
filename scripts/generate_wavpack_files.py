"""Owned complete native WavPack files, paired correction, prefixes and trailers.
Imports only owned synthetic encoders. No private input, old generated fixture,
external codec/process or network is read. Associated videos are codec controls;
the native .wv/.wvc files themselves reproduce the missing file-opening path.
"""
import json,struct
from pathlib import Path
from generate_wavpack_false_stereo import block
from generate_wavpack_float import rewrite,parts,metadata
from generate_wavpack_multichannel import compact,movie
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/wavpack-files'
def total(raw,value):
 out=bytearray(raw);at=0
 while at<len(out):
  size=int.from_bytes(out[at+4:at+8],'little')+8;out[at+11]=0;out[at+12:at+16]=value.to_bytes(4,'little');at+=size
 return bytes(out)
def generate():
 files={};cases=[]
 sources=[('false','generate_wavpack_false_stereo'),('float','generate_wavpack_float'),('multi','generate_wavpack_multichannel'),('rate','generate_wavpack_custom_rate'),('hybrid','generate_wavpack_hybrid'),('correction','generate_wavpack_correction'),('dsd','generate_wavpack_dsd'),('float-hybrid','generate_wavpack_float_hybrid')]
 for source,module in sources:
  generated=__import__(module).generate();manifest=json.loads(generated['manifest.json'])
  for case in manifest['cases']:
   name=source+'-'+case['name'];fn=name+'.wv';paired='correction'in case['frames'][0];frames=[];offset=0
   main=b''.join(generated[f['file']]for f in case['frames']);files[fn]=main
   correction=name+'.wvc'if paired else None
   if paired:files[correction]=b''.join(generated[f['correction']]for f in case['frames'])
   for f in case['frames']:
    raw=generated[f['file']];frames.append(dict(offset=offset,length=len(raw),samples=int.from_bytes(raw[20:24],'little')));offset+=len(raw)
   key='pcm'if source=='dsd'else'oracle';files[name+'.f32le']=generated[case[key]]
   lossy=case.get('lossy');files[name+'-lossy.f32le']=generated[lossy]if lossy else generated[case[key]]
   files[name+'.mkv']=generated[case['file']]
   channels=case.get('channels',2);rate=case.get('rate',8000);n=sum(f['samples']for f in frames)
   cases.append(dict(name=name,file=fn,correction=correction,oracle=name+'.f32le',lossy=name+'-lossy.f32le',video=name+'.mkv',channels=channels,rate=rate,samples=n,frames=frames,unknown=False))
 # Prefix/trailer variants preserve codec words and sample windows.
 base=cases[0];raw=files[base['file']]
 id3body=b'owned WavPack prefix';size=len(id3body);synch=bytes([(size>>21)&127,(size>>14)&127,(size>>7)&127,size&127]);head=b'ID3\x04\0\x10'+synch;id3=head+id3body+b'3DI'+head[3:]
 item=struct.pack('<II',5,0)+b'Title\0owned'
 def ape(header):
  flags=0x80000000 if header else 0;tail=b'APETAGEX'+struct.pack('<IIII',2000,32+len(item),1,flags)+bytes(8)
  prefix=b'APETAGEX'+struct.pack('<IIII',2000,32+len(item),1,flags|0x20000000)+bytes(8)if header else b''
  return prefix+item+tail
 meta=b'wvpk'+struct.pack('<IHBBIIIII',24+len(metadata(0x21,b'owned wrapper')),0x410,0,0,0xffffffff,0,0,0,0)+metadata(0x21,b'owned wrapper')
 for name,prefix,suffix,unknown in [('id3-ape',id3,ape(False)+b'TAG'+bytes(125),False),('ape-header',b'',ape(True),False),('metadata-blocks',meta,meta,False),('unknown-total',b'',b'',True)]:
  c={**base,'name':name,'file':name+'.wv','unknown':unknown};files[c['file']]=prefix+(total(raw,0xffffffff)if unknown else raw)+suffix;c['frames']=[{**f,'offset':f['offset']+len(prefix)}for f in base['frames']];cases.append(c)
 # A TAG-like suffix inside a valid block is data, not an ID3 trailer.
 frames=base['frames'];last=raw[frames[-1]['offset']:];last=rewrite(last,parts(last)+[(0x21,b'TAG'+bytes(125))]);c={**base,'name':'tag-inside-block','file':'tag-inside-block.wv','frames':[dict(f)for f in frames]};c['frames'][-1]['length']=len(last);files[c['file']]=raw[:frames[-1]['offset']]+last;cases.append(c)
 # Native frame lengths are independent, including sub-filter-size blocks.
 counts=[1,7,160];frames=[];raws=[];pcm=b'';at=0;pts=0
 for n in counts:
  b,p=block(16,8,False,n*50,mode='mono');flags=(int.from_bytes(b[24:28],'little')&~(15<<23))|(1<<23);b=rewrite(b,[(t,v)for t,v in parts(b)if t!=0x27],flags);b=bytearray(b);b[12:16]=sum(counts).to_bytes(4,'little');b[16:20]=pts.to_bytes(4,'little');b=bytes(b);raws.append(b);frames.append(dict(offset=at,length=len(b),samples=n));pcm+=p;at+=len(b);pts+=n
 files['variable.wv']=b''.join(raws);files['variable.f32le']=pcm;files['variable.mkv']=movie([compact([b])for b in raws],8000,1,16)
 cases.append(dict(name='variable',file='variable.wv',correction=None,oracle='variable.f32le',lossy='variable.f32le',video='variable.mkv',channels=1,rate=8000,samples=sum(counts),frames=frames,unknown=False))
 invalid=[]
 def bad(name,data,message):
  fn=name+'.wv';files[fn]=data;invalid.append(dict(file=fn,message=message))
 bad('truncated-header',raw[:20],'truncated WavPack native block header')
 bad('truncated-body',raw[:-1],'native block exceeds file')
 first=base['frames'][0]['length'];b=bytearray(raw);b[first+16:first+20]=(999).to_bytes(4,'little');bad('index-gap',bytes(b),'sample indices are not contiguous')
 b=bytearray(raw);b[12:16]=(base['samples']+1).to_bytes(4,'little');b[first+12:first+16]=(base['samples']+1).to_bytes(4,'little');third=first+base['frames'][1]['length'];b[third+12:third+16]=(base['samples']+1).to_bytes(4,'little');bad('total-long',bytes(b),'ends before declared total')
 bad('unknown-truncated',total(raw,0xffffffff)[:-1],'native block exceeds file')
 bad('trailing-junk',raw+bytes(40),'native block tag')
 b=bytearray(raw);b[4:8]=(0xffffffff).to_bytes(4,'little');bad('block-size',bytes(b),'native block length')
 bad('id3-size',b'ID3\x04\0\0\x7f\x7f\x7f\x7f'+raw,'ID3 prefix exceeds file')
 bad('ape-size',raw+b'APETAGEX'+struct.pack('<IIII',2000,0xffffffff,1,0)+bytes(8),'APE size exceeds file')
 paircase=next(c for c in cases if c['correction']);paired=files[paircase['correction']];b=bytearray(paired);b[16]^=1;files['wrong-pair.wvc']=bytes(b);files['short-pair.wvc']=paired[:int.from_bytes(paired[4:8],'little')+8];files['extra-pair.wvc']=paired+paired[-(int.from_bytes(paired[4:8],'little')+8):]
 files['manifest.json']=(json.dumps(dict(provenance=__doc__,cases=cases,invalid=invalid,pair_case=paircase['name']),indent=2)+'\n').encode();return files
if __name__=='__main__':
 OUT.mkdir(exist_ok=True)
 for n,b in generate().items():(OUT/n).write_bytes(b)
