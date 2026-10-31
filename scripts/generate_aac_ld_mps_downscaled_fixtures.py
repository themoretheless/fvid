#!/usr/bin/env python3
"""Own short LD/ELD cores plus ordinary LD-MPS; explicit offline generation."""
import json,math
from pathlib import Path
from generate_aac_ld_downscaled_fixtures import Source,short_pcm,interleave
from generate_aac_ld_layout_fixtures import LAYOUTS,geometry,body as core_body
from generate_aac_eld_fixtures import generate as eld_generate,mkv
from generate_aac_eld_short_fixtures import short_window
from generate_aac_eld_downscale_info_fixtures import prefix,extension
from generate_aac_ld_mps_stp_fixtures import config,body
from generate_aac_ld_mps_inband_fixtures import raw
from generate_aac_ld_mps_transport_fixtures import envelope,length
from generate_he_aac_packet_fixtures import field,frequency,packed,video_fixture
ROOT=Path(__file__).resolve().parents[1];FIX=ROOT/'tests/fixtures/playback-errors';OUT=FIX/'aac-ld-mps-downscaled';FRAMES=32

def ld_core(n,cfg,factor):
 count,last=geometry(n,48000);elements,mapping,_=LAYOUTS[cfg];banks=[Source(n,last)for _ in mapping];spectra=[[]for _ in mapping];frames=[]
 for i in range(FRAMES):
  bits='';ch=0;tags={}
  for element in elements:
   tag=tags.get(element,0);tags[element]=tag+1;bits+=field(tag,4)+('0'if element==1 else'')
   for _ in range(2 if element==1 else 1):
    encoded,args=core_body(n,count,last,i%8,ch,lfe=element==3);bits+=encoded;values,_=banks[ch].run(*args);spectra[ch].append((values,args[2]));ch+=1
  frames.append(bits)
 gold=interleave([short_pcm(n,n//factor,b.positions,rows)for b,rows in zip(banks,spectra)],mapping);asc=packed(field(23,5)+frequency(48000)+field(cfg,4)+field(n==480,1)+'00'+'00');return asc,frames,gold

def main():
 OUT.mkdir(exist_ok=True);am=json.loads((FIX/'aac-ld-mps-assemble/manifest.json').read_text())['cases'];cfgs=(FIX/'aac-ld-mps-assemble/configs.bin').read_bytes();blob=bytearray();cases=[];controls=[]
 def rows(frames):
  out=[]
  for bits in frames:
   wire=packed(bits);out.append(dict(offset=len(blob),bytes=len(wire),core_bits=len(bits)));blob.extend(wire)
  return out
 for profile in [23,39]:
  for n in [480,512]:
   for factor in [2,3,4,8]:
    if n%factor:continue
    rate=48000//factor;sn=n//factor;group=32//math.gcd(32,sn)
    while group*sn//32<4:group*=2
    assert FRAMES%group==0;slots=group*sn//32;cores={};eld_ends={}
    for cfg in [1,2,6]:
     if profile==23:asc,frames,gold=ld_core(n,cfg,factor)
     else:
      files=eld_generate(include_payload_bits=True,specs=[(n,48000,cfg,'high',0,0,True)],output_samples=sn,synthesis_window=short_window,frame_count=FRAMES);c=json.loads(files['manifest.json'])['cases'][0];frames=[r['bits']for r in c['frames']];gold=files[c['name']+'.f32le'];head,tail=prefix(c['asc']);eld_ends[cfg]=(head,tail);asc=packed(head+extension(rate,True)+tail)
     inputs=len(LAYOUTS[cfg][1]);name=f'aot{profile}-n{n}-ds{factor}-c{cfg}';wire_rows=rows(frames);base=dict(name=name,n=n,factor=factor,profile=profile,configuration=cfg,channels=inputs,rate=rate,source_rate=48000,container_rate=rate,container_frame_samples=sn,samples=FRAMES*sn,pcm_offset=0,asc=asc.hex(),frames=wire_rows,out_n=sn,slots=slots,bands=32,video=name+'-core.mp4',golden=name+'-core.f32le');(OUT/base['video']).write_bytes(video_fixture([base],blob,inputs,write=False)['data']);(OUT/base['golden']).write_bytes(gold);controls.append(base);cores[cfg]=(asc,frames,base)
    for tree in range(8):
     cfg=1 if tree in[0,1,7]else 2 if tree in[2,3,4]else 6;asc,frames,base=cores[cfg];c=next(c for c in am if c['tree']==tree and c['variant']==1 and c['bands']==23);ssc=config(c,48000,slots,cfgs[c['config_offset']:c['config_offset']+c['config_bytes']],0);ssc=packed(frequency(rate)+raw(ssc)[4:])
     if profile==23:spatial_asc=packed(raw(asc)[:5+4+4+1+2+2]+field(0x7cc,11)+'1'+field(len(ssc),8)+raw(ssc))
     else:head,tail=eld_ends[cfg];spatial_asc=packed(head+extension(rate,True)+field(2,4)+length(len(ssc))+raw(ssc)+tail)
     suffix=['']*FRAMES;reference=[]
     for i in range(FRAMES//group):
      payload,_=body(c,i,slots,'alternating',False);suffix[(i+1)*group-1]=envelope(9,1 if i==0 else 0,3,(bytes([len(ssc)])+ssc if i==0 else b'')+payload,True);reference.append(payload.hex())
     packets=[bits+tail for bits,tail in zip(frames,suffix)];wire_rows=rows(packets);name=f'aot{profile}-n{n}-ds{factor}-tree{tree}';out=[6,6,6,8,8,8,8,2][tree];case=dict(base,name=name,asc=spatial_asc.hex(),frames=wire_rows,tree=tree,channels=out,input_channels=base['channels'],group=group,slots=slots,ssc=ssc.hex(),reference=reference,core_gold=base['golden'],video=name+'.mp4',file=name+'.mkv',live_prefix_packets=group+2,edges=[group*sn,2*group*sn,3*group*sn]);(OUT/case['video']).write_bytes(video_fixture([case],blob,out,write=False)['data']);(OUT/case['file']).write_bytes(mkv(spatial_asc,[packed(bits)for bits in packets],rate,sn,out));cases.append(case)
 (OUT/'packets.bin').write_bytes(blob);(OUT/'manifest.json').write_text(json.dumps(dict(cases=cases,controls=controls,exports=[dict(c,video=c['file'],name=c['name']+'-matroska')for c in cases],scope='Own downscaled AAC-LD/ELD without SBR plus ordinary LD-MPS;analytical short core PCM and separately qualified spatial composition,not whole-codec normative conformance.'),separators=(',',':'))+'\n');print(len(cases),'spatial programmes;',len(controls),'analytical short-core controls')
if __name__=='__main__':main()
