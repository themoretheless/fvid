"""Invented IMA WAV blocks and analytic PCM; separate from offline regressions."""
from pathlib import Path
import json,struct
from generate_rf64_playback import generate as reference_files, build as rf64, chunk
from generate_wave64_playback import build as wave64
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/wave-ima-playback'
def riff(fmt,raw,frames,mode):
 body=b'WAVE'+chunk(b'fmt ',fmt)
 if mode!='nofact':body+=chunk(b'fact',struct.pack('<I',frames))
 body+=chunk(b'data',raw)+chunk(b'JUNK',b'after-data')
 return b'RIFF'+struct.pack('<I',len(body))+body

def generate():
 parent=reference_files();files={n:parent[n]for n in ['neutral-video.mp4','frame-0.yuv16','frame-1.yuv16','frame-2.yuv16']};cases=[]
 for channels in [1,2]:
  for per_channel,blocks in [(8,259),(2048,2)]:
   align=per_channel*channels;spb=1+(align-4*channels)*2//channels;rate=8000
   fmt=struct.pack('<HHIIHHHH',17,channels,rate,rate*align//spb,align,4,2,spb)
   raw=b'';samples=[]
   for n in range(blocks):
    starts=[-2000+(n%11)*101+ch*3000 for ch in range(channels)]
    header=b''.join(struct.pack('<hBB',p,4,0)for p in starts)
    if channels==1:payload=bytes([0])* (align-4)
    else:payload=(bytes(4)+bytes([0x88])*4)*((align-8)//8)
    raw+=header+payload
    for i in range(spb):
     for ch in range(channels):samples.append(starts[ch]+min(i,4)*(1 if ch==0 else -1))
   for mode in ['full','trim','nofact']:
    frames=blocks*spb-(min(spb-1,7)if mode=='trim'else 0)
    key=f'c{channels}-b{align}-{mode}';pcm=b''.join(struct.pack('<h',v)for v in samples[:frames*channels]);gold=b''.join(struct.pack('<f',v/32768)for v in samples[:frames*channels])
    control=key+'.pcm.wav';files[control]=riff(struct.pack('<HHIIHH',1,channels,rate,rate*channels*2,channels*2,16),pcm,frames,'nofact');files[key+'.f32le']=gold;files[key+'.encoded']=raw
    for kind in ['riff','rf64','wave64']:
     name=key+'-'+kind;file=name+'.wav'
     files[file]=riff(fmt,raw,frames,mode)if kind=='riff'else rf64(fmt,raw,frames,'nofact'if mode=='nofact'else'base')if kind=='rf64'else wave64(fmt,raw,frames,'nofact'if mode=='nofact'else'base')
     cases.append(dict(name=name,file=file,control=control,oracle=key+'.f32le',encoded=key+'.encoded',frames=frames,channels=channels,rate=rate,data_bytes=len(raw),block_bytes=align,block_frames=spb,mode=mode,kind=kind,declared_frames=frames if mode!='nofact'or kind=='rf64'else None))
 fmt=struct.pack('<HHIIHHHH',17,2,8000,14222,16,4,2,9);raw=struct.pack('<hBBhBB',10,4,0,-10,4,0)+bytes(8);bad=[]
 def emit(name,f,p,needle,unsupported=False):files[name+'.wav']=riff(f,p,9,'full');bad.append(dict(file=name+'.wav',needle=needle,unsupported=unsupported))
 for name,at,value,needle in [('bad-width',14,3,'IMA'),('bad-align',12,14,'IMA'),('bad-samples',18,8,'samples-per-block'),('bad-extension',16,1,'extension')]:
  f=bytearray(fmt);struct.pack_into('<H',f,at,value);emit(name,bytes(f),raw,needle)
 f=bytearray(fmt);struct.pack_into('<H',f,2,3);emit('unsupported-channels',bytes(f),raw,'IMA',True)
 emit('partial-block',fmt,raw[:-1],'incomplete IMA')
 b=bytearray(riff(fmt,raw,9,'full'));fact=b.index(b'fact');struct.pack_into('<I',b,fact+8,10);files['overstated-fact.wav']=bytes(b);bad.append(dict(file='overstated-fact.wav',needle='IMA fact',unsupported=False))
 source=files['c1-b8-full-riff.wav'];at=source.index(b'data')+8;raw=files['c1-b8-full.encoded']
 files['truncated-block-prefix.wav']=source[:at+len(raw)-8]
 files['truncated-block-prefix.f32le']=files['c1-b8-full.f32le'][:-9*4]
 corrupt=bytearray(source);corrupt[at+2]=89;files['bad-step-index.wav']=bytes(corrupt)
 stream=bytearray(source[:at+len(raw)]);struct.pack_into('<I',stream,4,len(stream)-8);struct.pack_into('<I',stream,at-4,0xffffffff)
 files['unknown-data-complete.wav']=bytes(stream)
 fact=stream.index(b'fact');struct.pack_into('<I',stream,fact+8,2331+9);files['unknown-data-short.wav']=bytes(stream)
 files['manifest.json']=(json.dumps(dict(provenance='Invented step-index4 IMA blocks: zero/sign-only nibbles yield four unit steps then a constant predictor; small and >2048-frame blocks, mono/stereo, fact tail and absent fact. RIFF/RF64/Wave64 use identical encoded bytes. Independent literal PCM controls. No encoder,private input or network.',cases=cases,bad=bad),indent=2)+'\n').encode();return files
if __name__=='__main__':
 OUT.mkdir(parents=True,exist_ok=True)
 for n,b in generate().items():(OUT/n).write_bytes(b)
