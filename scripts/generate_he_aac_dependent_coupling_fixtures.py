#!/usr/bin/env python3
"""Own nonzero dependent CCE/SBR videos and direct-cosine core PCM. Offline only."""
import json,math,re,struct,hashlib
from generate_he_aac_packet_fixtures import DEST,field,frequency,packed,video_fixture
from generate_aac_sbr_dsp_fixtures import payload as mono_payload
from generate_he_aac_stereo_fixtures import payload as pair_payload
from generate_aac_sbr_frequency_oracles import tables
ROOT=DEST.parents[2]
source=(ROOT/'crates/fvid-media/src/owned_aac/aac_huffman_tables.rs').read_text()
def table(name):
 text=re.search(r'const '+name+r':[^=]+ = \[(.*?)\];',source,re.S)[1]
 return [int(n,0) for n in re.findall(r'0x[0-9a-fA-F]+|\d+',text)]
codes=table('SPECTRUM_CODEBOOK1_CODES');lens=table('SPECTRUM_CODEBOOK1_LENS');sc=table('SCF_CODEBOOK_CODES');sl=table('SCF_CODEBOOK_LENS')
def word(n):return field(sc[n],sl[n])
def program(prefix,stereo,independent=False):
 bits=prefix+field(0,4)+field(1,2)+frequency(24000)+field(1,4)+field(0,4)+field(0,4)+field(0,2)+field(0,3)+field(1,4)+'000'+field(stereo,1)+field(0,4)+field(independent,1)+field(1,4)
 return bits+'0'*(-len(bits)%8)+field(0,8)
def config(slots,rate,kind,stereo,independent=False):
 ga=field(slots==15,1)+'00';prefix=(field(5,5)+frequency(24000)+'0000'+frequency(rate)+field(2,5)+ga) if kind=='explicit' else field(2,5)+frequency(24000)+'0000'+ga
 bits=program(prefix,stereo,independent)
 if kind=='sync':bits+=field(0x2b7,11)+field(5,5)+'1'+frequency(rate)
 return packed(bits)
def info():return '0'+'00'+'0'+field(1,6)+'0'
# Silent target has one active band and first-order TNS (reflection sin(pi/7)).
def target(common):
 return field(140,8)+('' if common else info())+field(0,4)+field(1,5)+'0'+'1'+field(1,2)+'0'+field(49,6)+field(1,5)+'00'+field(1,3)+'0'
def packet(frame,stereo,point,raw,missing=False,cce_raw=b''):
 bits=program('101',stereo)
 bits+=('001'+'0000'+'1'+info()+'00'+target(True)*2) if stereo else ('000'+'0000'+target(False))
 if raw:
  size=len(raw);bits+='110'+(field(size,4) if size<15 else '1111'+field(size-14,8))+''.join(field(b,8) for b in raw)
 bits+='010'+field(1,4)+'0'+field(0,3)+field(stereo,1)+field(missing,4)+(field(3,2) if stereo else '')+field(point,1)+'0'+'00'
 bits+=field(140,8)+info()+field(1,4)+field(1,5)+word(60)+'000'
 index=80 if frame%2==0 else 0;bits+=field(codes[index],lens[index])
 if stereo:bits+='1'+word(64)
 if cce_raw:
  size=len(cce_raw);bits+='110'+(field(size,4) if size<15 else '1111'+field(size-14,8))+''.join(field(b,8) for b in cce_raw)
 return packed(bits+'111')
# Independent O(N^2) IMDCT/window/overlap, not the production FFT synthesis.
def core(n,stereo,point):
 channels=2 if stereo else 1;overlap=[[0.]*n for _ in range(channels)];out=bytearray()
 for frame in range(6):
  coeff=[1024.*(1 if frame%2==0 else -1)]*4
  if point==0:
   previous=0.
   for k in range(4):coeff[k]-=math.sin(math.pi/7)*previous;previous=coeff[k]
  signals=[]
  for c in range(channels):
   gain=2**(-.5) if c else 1.
   block=[sum(v*gain*math.cos(math.pi/n*(t+.5+n/2)*(k+.5)) for k,v in enumerate(coeff))*2/n/65536*math.sin(math.pi/(2*n)*(t+.5)) for t in range(2*n)]
   signals.append([block[i]+overlap[c][i] for i in range(n)]);overlap[c]=block[n:]
  for i in range(n):
   for c in range(channels):out.extend(struct.pack('<f',signals[c][i]))
 return bytes(out)
def main():
 blob=bytearray();pcm=bytearray();cases=[];nhigh=len(tables(10,27,0,False,0,0)[1])-1
 for slots in [15,16]:
  for stereo in [False,True]:
   for point in [0,1]:
    values=core(slots*64,stereo,point);descriptor=[len(pcm),len(values)//4];pcm.extend(values)
    for fill in [False,True]:
     frames=[]
     for frame in range(6):
      raw=(pair_payload(nhigh,2,True,frame,False) if stereo else mono_payload(nhigh,2,True,frame)) if fill else b''
      data=packet(frame,stereo,point,raw);frames.append(dict(offset=len(blob),bytes=len(data),sbr=raw.hex()));blob.extend(data)
     for rate,bands in [(24000,32),(48000,64)]:
      for kind in ['explicit','sync','implicit']:
       if kind=='implicit' and bands==32:continue
       case=dict(slots=slots,bands=bands,frames=frames,asc=config(slots,rate,kind,stereo).hex(),pcm_offset=0,samples=slots*bands*12)
       video=video_fixture([case],blob,channels=2 if stereo else 1,filename=f'he-aac-dependent-sbr-{slots*64}-{int(stereo)}-{point}-{int(fill)}-{kind}-{rate}-synthetic.mp4') if bands==64 else None
       case['independent_asc']=config(slots,rate,kind,stereo,True).hex()
       cases.append(dict(case,channels=2 if stereo else 1,point=point,kind=kind,core_pcm=descriptor,output_rate=rate,video=video))
 invalid=[]
 for stereo in [False,True]:
  frames=[]
  for frame in range(3):
   raw=pair_payload(nhigh,2,True,frame,False) if stereo else mono_payload(nhigh,2,True,frame)
   data=packet(frame,stereo,0,raw,missing=frame==1)
   frames.append(dict(offset=len(blob),bytes=len(data),sbr=raw.hex()));blob.extend(data)
  case=dict(slots=16,bands=64,frames=frames,asc=config(16,48000,'explicit',stereo).hex(),pcm_offset=0,samples=6144)
  video=video_fixture([case],blob,channels=2 if stereo else 1,filename=f'he-aac-dependent-sbr-missing-target-{int(stereo)}-synthetic.mp4')
  invalid.append(dict(case,video=video,error='AAC coupling target is absent'))
 (DEST/'he-aac-dependent-sbr-packets.bin').write_bytes(blob);(DEST/'he-aac-dependent-sbr-core.f32le').write_bytes(pcm)
 (DEST/'he-aac-dependent-sbr-oracles.json').write_text(json.dumps(dict(kind='owned dependent CCE spectra; independent direct-cosine core PCM; SBR stage composition',cases=cases,invalid=invalid,packet_sha256=hashlib.sha256(blob).hexdigest(),core_sha256=hashlib.sha256(pcm).hexdigest()),indent=2)+'\n')
 print('dependent SBR cases',len(cases))
if __name__=='__main__':main()
