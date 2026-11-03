"""Invented MS ADPCM WAVE blocks with standard and extended predictors."""
from pathlib import Path
import json,struct
from generate_rf64_playback import generate as reference_files, build as rf64
from generate_wave64_playback import build as wave64
from generate_wave_ima_playback import riff
from generate_avi_audio_eof_regressions import chunk,listing,stream as avi_stream
from generate_vp9_chroma import frame
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/wave-ms-adpcm-playback'
STANDARD=[(256,0),(512,-256),(0,0),(192,64),(240,0),(460,-208),(392,-232)]
def fmt(channels,align,coefficients):
 frames=2+(align-7*channels)*2//channels
 return struct.pack('<HHIIHHHHH',2,channels,8000,8000*align//frames,align,4,4+4*len(coefficients),frames,len(coefficients))+b''.join(struct.pack('<hh',a,b)for a,b in coefficients)
def block(channels,align,number,extended):
 starts=[-1000+(number%11)*23+ch*2500 for ch in range(channels)];directions=[1 if ch==0 else -1 for ch in range(channels)]
 predictor=7 if extended else 1
 header=bytes([predictor])*channels+b''.join(struct.pack('<h',16)for _ in starts)+b''.join(struct.pack('<h',v+d)for v,d in zip(starts,directions))+b''.join(struct.pack('<h',v)for v in starts)
 frames=2+(align-7*channels)*2//channels
 samples=[v+(i%2 if extended else i)*d for i in range(frames)for v,d in zip(starts,directions)]
 return header+bytes(align-len(header)),samples

def generate():
 parent=reference_files();files={n:parent[n]for n in ['neutral-video.mp4','frame-0.yuv16','frame-1.yuv16','frame-2.yuv16']};cases=[]
 for channels in [1,2]:
  for per_channel,blocks in [(8,600),(2048,2)]:
   align=per_channel*channels;spb=2+(align-7*channels)*2//channels
   for table in ['standard','extended']:
    coefficients=STANDARD+([(0,256)]if table=='extended'else[]);format_bytes=fmt(channels,align,coefficients);raw=b'';samples=[]
    for n in range(blocks):
     b,p=block(channels,align,n,table=='extended');raw+=b;samples+=p
    for mode in ['full','trim','nofact']:
     frames=blocks*spb-(min(spb-1,7)if mode=='trim'else 0);key=f'c{channels}-b{align}-{table}-{mode}';control=key+'.pcm.wav'
     pcm=b''.join(struct.pack('<h',v)for v in samples[:frames*channels]);gold=b''.join(struct.pack('<f',v/32768)for v in samples[:frames*channels])
     files[control]=riff(struct.pack('<HHIIHH',1,channels,8000,8000*channels*2,channels*2,16),pcm,frames,'nofact');files[key+'.f32le']=gold;files[key+'.encoded']=raw;files[key+'.fmt']=format_bytes
     for kind in ['riff','rf64','wave64']:
      name=key+'-'+kind;file=name+'.wav';files[file]=riff(format_bytes,raw,frames,mode)if kind=='riff'else rf64(format_bytes,raw,frames,'nofact'if mode=='nofact'else'base')if kind=='rf64'else wave64(format_bytes,raw,frames,'nofact'if mode=='nofact'else'base')
      cases.append(dict(name=name,file=file,control=control,oracle=key+'.f32le',encoded=key+'.encoded',frames=frames,channels=channels,rate=8000,data_bytes=len(raw),block_bytes=align,block_frames=spb,mode=mode,table=table,setup=key+'.fmt',kind=kind,declared_frames=frames if mode!='nofact'or kind=='rf64'else None))
 coefficients=STANDARD+[(0,256)]*249;format_bytes=fmt(1,8,coefficients);raw=b'';samples=[]
 for n in range(2):
  b,p=block(1,8,n,True);raw+=bytes([255])+b[1:];samples+=p
 key='c1-b8-max-count-full';gold=b''.join(struct.pack('<f',v/32768)for v in samples);pcm=b''.join(struct.pack('<h',v)for v in samples)
 files[key+'.fmt']=format_bytes;files[key+'.encoded']=raw;files[key+'.f32le']=gold;files[key+'.pcm.wav']=riff(struct.pack('<HHIIHH',1,1,8000,16000,2,16),pcm,8,'nofact');files[key+'-riff.wav']=riff(format_bytes,raw,8,'full')
 cases.append(dict(name=key+'-riff',file=key+'-riff.wav',control=key+'.pcm.wav',oracle=key+'.f32le',encoded=key+'.encoded',setup=key+'.fmt',frames=8,channels=1,rate=8000,data_bytes=len(raw),block_bytes=8,block_frames=4,mode='full',table='max-count',kind='riff',declared_frames=8))
 format_bytes=fmt(2,16,STANDARD);raw,_=block(2,16,0,False);bad=[]
 def emit(name,f,p,needle,unsupported=False):files[name+'.wav']=riff(f,p,4,'full');bad.append(dict(file=name+'.wav',needle=needle,unsupported=unsupported))
 for name,at,value,needle in [('bad-width',14,3,'MS ADPCM'),('bad-align',12,13,'MS ADPCM'),('bad-samples',18,3,'samples-per-block'),('bad-extension',16,2,'extension'),('bad-coef-count',20,0,'coefficient'),('huge-coef-count',20,257,'coefficient'),('bad-standard-coef',22,255,'standard coefficient')]:
  f=bytearray(format_bytes);struct.pack_into('<H',f,at,value);emit(name,bytes(f),raw,needle)
 f=bytearray(format_bytes);struct.pack_into('<H',f,2,3);emit('unsupported-channels',bytes(f),raw,'MS ADPCM',True)
 emit('partial-block',format_bytes,raw[:-1],'incomplete MS ADPCM')
 emit('truncated-coefficients',format_bytes[:-1],raw,'coefficient table')
 b=bytearray(riff(format_bytes,raw,4,'full'));at=b.index(b'fact');struct.pack_into('<I',b,at+8,5);files['overstated-fact.wav']=bytes(b);bad.append(dict(file='overstated-fact.wav',needle='MS ADPCM fact',unsupported=False))
 source=files['c1-b8-standard-full-riff.wav'];at=source.index(b'data')+8;raw=files['c1-b8-standard-full.encoded'];files['c1-b8-full.f32le']=files['c1-b8-standard-full.f32le']
 files['truncated-block-prefix.wav']=source[:at+len(raw)-8];files['truncated-block-prefix.f32le']=files['c1-b8-standard-full.f32le'][:-4*4]
 b=bytearray(source);b[at]=7;files['bad-predictor.wav']=bytes(b)
 b=bytearray(source);struct.pack_into('<h',b,at+1,0);files['bad-delta.wav']=bytes(b)
 stream=bytearray(source[:at+len(raw)]);struct.pack_into('<I',stream,4,len(stream)-8);struct.pack_into('<I',stream,at-4,0xffffffff);files['unknown-data-complete.wav']=bytes(stream)
 fact=stream.index(b'fact');struct.pack_into('<I',stream,fact+8,2404);files['unknown-data-short.wav']=bytes(stream)
 # An unsupported extensible MS track must not hide a later playable PCM track.
 ext=struct.pack('<HHIIHHHHII',65534,2,8000,32000,16,4,22,4,3,2)+bytes.fromhex('00001000800000aa00389b71')
 msraw,_=block(2,16,0,False);files['unsupported-extensible-ms.wav']=riff(ext,msraw,4,'full')
 values=[[-1000+i,1000-i]for i in range(16)];pcm=b''.join(struct.pack('<h',v)for row in values for v in row);gold=b''.join(struct.pack('<f',v/32768)for row in values for v in row)
 valid=struct.pack('<HHIIHHHHII',65534,2,8000,32000,4,16,22,16,3,1)+bytes.fromhex('00001000800000aa00389b71')
 ah=bytearray(avi_stream(b'auds',bytes(4),1,8000,16,64));struct.pack_into('<I',ah,44,4)
 audio=listing(b'strl',chunk(b'strh',ah)+chunk(b'strf',ext))+listing(b'strl',chunk(b'strh',ah)+chunk(b'strf',valid))
 coded=frame(8,'420',16,16);vf=struct.pack('<IiiHH4sIiiII',40,16,16,1,24,b'VP90',len(coded),0,0,0,0)
 video=listing(b'strl',chunk(b'strh',avi_stream(b'vids',b'VP90',1,50,1,len(coded)))+chunk(b'strf',vf));main=struct.pack('<14I',20000,0,0,0,1,0,3,max(len(coded),64),16,16,0,0,0,0)
 body=b'AVI '+listing(b'hdrl',chunk(b'avih',main)+video+audio)+listing(b'movi',chunk(b'00dc',coded)+chunk(b'01wb',msraw)+chunk(b'02wb',pcm))
 files['after-unsupported-extensible-ms.avi']=b'RIFF'+struct.pack('<I',len(body))+body;files['after-unsupported-extensible-ms.f32le']=gold
 files['manifest.json']=(json.dumps(dict(provenance='Invented MS ADPCM blocks with zero residual nibbles: standard predictor1 yields an exact ramp, added predictor7=(0,256) alternates two seeds. Standard seven coefficient pairs plus one extension, mono/stereo, small/large blocks and sample-count tails. Independent literal PCM; no encoder,network or private media.',cases=cases,bad=bad),indent=2)+'\n').encode();return files
if __name__=='__main__':
 OUT.mkdir(parents=True,exist_ok=True)
 for n,v in generate().items():(OUT/n).write_bytes(v)
