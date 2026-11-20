#!/usr/bin/env python3
"""Own spatial-bank LD-SBR QMF controls. Explicit offline generation only."""
import cmath,json,math,struct
from pathlib import Path
from generate_aac_eld_fixtures import generate as core_generate
from generate_aac_eld_sbr_fixtures import reconstruct,sbr_bits
from generate_aac_ld_layout_fixtures import LAYOUTS
from generate_aac_sbr_frequency_oracles import tables
from generate_aac_sbr_dsp_fixtures import header
from generate_aac_ld_mps_transport_fixtures import envelope,field,length,packed
from generate_he_aac_packet_fixtures import frequency,video_fixture
ROOT=Path(__file__).resolve().parents[1]
OUT=ROOT/'tests/fixtures/playback-errors/aac-eld-spatial-sbr'
def raw(data):return ''.join(field(v,8)for v in data)
def analysis(pcm,window,bands):
 c=[(a+b)/2 for a,b in zip(window[::2],window[1::2])]
 # Absolute-time convolution with independently authored LD modulation, not
 # production history shifting or the ordinary CLDFB analysis oracle.
 weights=[[2*c[lag]*cmath.exp(1j*math.pi*(k+.5)*(2*(lag%64)+65)/64)for lag in range(320)]for k in range(bands)]
 return [[sum(pcm[t*32+31-lag]*w[lag]for lag in range(min(320,t*32+32)))*32768 for w in weights]for t in range(len(pcm)//32)]
def main():
 OUT.mkdir(exist_ok=True)
 cores=core_generate(include_payload_bits=True,specs=[(n,24000,c,'base',0,0,False)for n in(480,512)for c in(1,2,6)])
 cm=json.loads(cores['manifest.json'])['cases'];noise=[complex(*struct.unpack_from('<dd',(ROOT/'tests/fixtures/playback-errors/aac-sbr-noise-protocol.f64le').read_bytes(),i*16))for i in range(512)]
 window=json.loads((ROOT/'tests/fixtures/playback-errors/aac-ld-qmf/window.json').read_text())
 mps=ROOT/'tests/fixtures/playback-errors/aac-ld-mps-assemble';mm=json.loads((mps/'manifest.json').read_text())['cases'];configs=(mps/'configs.bin').read_bytes();parameters=(mps/'packets.bin').read_bytes()
 blob=bytearray();expected=bytearray();cases=[];errors=[]
 for core in cm:
  n=core['n'];cfg=core['configuration'];channels=core['channels'];slots=n//32;elements,mapping,_=LAYOUTS[cfg]
  (OUT/(core['name']+'.input.f32le')).write_bytes(cores[core['name']+'.f32le'])
  original=struct.unpack('<'+'f'*(len(cores[core['name']+'.f32le'])//4),cores[core['name']+'.f32le'])
  lfe=mapping[sum(2 if e==1 else 1 for e in elements[:elements.index(3)])]if 3 in elements else None
  cache={bands:[analysis(original[ch::channels],window,bands)for ch in range(channels)]for bands in(10,15,32)}
  modes=['base','wide']if cfg==6 else ['base','two','four','coarse','low','transient0','transient8','invf','smoothing','harmonic','header','header-switch','wide']+(['coupled','coupled-pan']if cfg==2 else[])
  for mode in modes:
   kx,stop=(15,45)if mode=='wide'else(10,27);_,high,low,_=tables(kx,stop,0,False,0,0)
   h=header(0,mode!='smoothing')
   if mode=='wide':h=h[:1]+field(7,4)+field(15,4)+h[9:]
   rows=[];stored=max(32,stop);eo=len(expected)
   for ch in range(channels):
    if ch==lfe:qmf=[row+[0j]*32 for row in cache[32][ch]]
    else:qmf=reconstruct(cache[kx][ch],slots,noise,mode,ch,(15,45,list(range(1,15))*2+[None,None])if mode=='wide'else None)
    rows.append(qmf)
    for row in qmf:
     for v in row[:stored]:expected.extend(struct.pack('<dd',v.real/32768,v.imag/32768))
   for double in(False,True):
    bands=64 if double else 32;rate=24000*(1+double);tree={1:7,2:2,6:5}[cfg]
    spatial=next(c for c in mm if c['tree']==tree and c['variant']==1)
    sc=configs[spatial['config_offset']:spatial['config_offset']+spatial['config_bytes']];text=raw(sc);sc=packed(field(3 if double else 6,4)+field(slots-1,5)+text[9:])
    for protected in ((False,True)if mode=='base'else(True,)):
     config=packed(field(31,5)+field(7,6)+frequency(24000)+field(cfg,4)+field(n==480,1)+'000'+'1'+field(double,1)+field(protected,1)+h*sum(e!=3 for e in elements)+field(2,4)+length(len(sc))+raw(sc)+'0000'+'00')
     frames=[]
     for i,c in enumerate(core['frames']):
      sbr=''.join(sbr_bits(2 if e==1 else 1,len(high)-1,len(low)-1,slots,i,protected,mode)for e in elements if e!=3)
      f=spatial['frames'][i%3];body=parameters[f['offset']:f['offset']+f['bytes']]
      packet=packed(c['bits']+sbr+envelope(9,0,3,body,True));frames.append(dict(offset=len(blob),bytes=len(packet),core_bits=len(c['bits']),sbr_bits=len(sbr)));blob.extend(packet)
     name=f'n{n}-c{cfg}-{mode}-x{1+double}-crc{int(protected)}'
     case=dict(name=name,n=n,configuration=cfg,channels=channels,mapping=mapping,mode=mode,bands=bands,slots=slots,rate=rate,asc=config.hex(),input=core['name']+'.input.f32le',expected_offset=eo,expected_bands=stored,frames=frames,container_rate=rate,container_frame_samples=n*(1+double),samples=12*n*(1+double),pcm_offset=0)
     (OUT/(name+'.mp4')).write_bytes(video_fixture([case],blob,channels,write=False)['data']);cases.append(case)
  # These sources reach a specific CRC/truncation error in spatial-bank SBR;
  # native playback must report the same bounded syntax error.
  if cfg==1:
   base=next(c for c in cases if c['n']==n and c['configuration']==cfg and c['mode']=='base'and c['bands']==64 and c['name'].endswith('crc1'))
   spatial=next(c for c in mm if c['tree']==7 and c['variant']==1)
   for error in('CRC','truncated'):
    frames=[]
    for i,c in enumerate(core['frames']):
     sbr=sbr_bits(1,16,8,slots,i,True)
     if i==3 and error=='CRC':sbr=('1'if sbr[0]=='0'else'0')+sbr[1:]
     f=spatial['frames'][i%3];body=parameters[f['offset']:f['offset']+f['bytes']]
     packet=packed(c['bits']+sbr+envelope(9,0,3,body,True))
     if i==3 and error=='truncated':packet=packed(c['bits']+sbr)[:-1]
     frames.append(dict(offset=len(blob),bytes=len(packet),core_bits=len(c['bits']),sbr_bits=len(sbr)));blob.extend(packet)
    case=dict(base,name=f'n{n}-{error}',frames=frames,error=error)
    (OUT/(case['name']+'.mp4')).write_bytes(video_fixture([case],blob,channels,write=False)['data']);errors.append(case)
 # Fill remaining standard spatial trees without changing the SBR oracle.
 for tree,cfg in [(0,1),(1,1),(3,2),(4,2),(6,6)]:
  spatial=next(c for c in mm if c['tree']==tree and c['variant']==1)
  for n in (480,512):
   for double in (False,True):
    base=next(c for c in cases if c['n']==n and c['configuration']==cfg and c['mode']=='base' and c['bands']==(64 if double else 32) and c['name'].endswith('crc1'))
    elements,_,_=LAYOUTS[cfg];slots=n//32
    sc=configs[spatial['config_offset']:spatial['config_offset']+spatial['config_bytes']]
    sc=packed(field(3 if double else 6,4)+field(slots-1,5)+raw(sc)[9:])
    config=packed(field(31,5)+field(7,6)+frequency(24000)+field(cfg,4)+field(n==480,1)+'000'+'1'+field(double,1)+'1'+header(0,True)*sum(e!=3 for e in elements)+field(2,4)+length(len(sc))+raw(sc)+'0000'+'00')
    frames=[]
    for i,c in enumerate(base['frames']):
     prefix=raw(blob[c['offset']:c['offset']+c['bytes']])[:c['core_bits']+c['sbr_bits']]
     f=spatial['frames'][i%3];body=parameters[f['offset']:f['offset']+f['bytes']]
     packet=packed(prefix+envelope(9,0,3,body,True))
     frames.append(dict(c,offset=len(blob),bytes=len(packet)));blob.extend(packet)
    case=dict(base,name=base['name']+f'-tree{tree}',asc=config.hex(),frames=frames)
    (OUT/(case['name']+'.mp4')).write_bytes(video_fixture([case],blob,cfg,write=False)['data']);cases.append(case)
 (OUT/'packets.bin').write_bytes(blob);(OUT/'expected.f64le').write_bytes(expected)
 (OUT/'manifest.json').write_text(json.dumps(dict(scope='Spatial-bank LD-SBR normalized QMF acceptance and specific error controls; native coupled AAC-ELD/LD-MPS playback acceptance.',cases=cases,errors=errors,provenance='Own ELD bits/core cosine controls, absolute-time LD-QMF convolution, SBR entropy/CRC and scalar HF/gain/noise equations. No private media or codec parameter sets, external codec, process or network.'),separators=(',',':'))+'\n')
 print(len(cases),len(errors),len(expected),'QMF bytes')
if __name__=='__main__':main()
