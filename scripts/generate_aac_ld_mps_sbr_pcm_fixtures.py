#!/usr/bin/env python3
"""Own downscaled LD-SBR PCM into LD-MPS. Offline, explicit generation only."""
import json,struct,math
from pathlib import Path
from functools import lru_cache
from generate_aac_eld_fixtures import generate as core,mkv
from generate_aac_eld_short_fixtures import short_window
from generate_aac_eld_downscale_info_fixtures import extension,prefix as eld_prefix
from generate_aac_eld_cldfb_downscaled_fixtures import analysis,synthesis
from generate_aac_eld_sbr_fixtures import reconstruct,sbr_bits
from generate_aac_sbr_frequency_oracles import tables
from generate_aac_ld_layout_fixtures import LAYOUTS
from generate_aac_sbr_dsp_fixtures import header
from generate_aac_ld_mps_stp_fixtures import config,body
from generate_aac_ld_mps_inband_fixtures import raw
from generate_aac_ld_mps_transport_fixtures import envelope,length
from generate_he_aac_packet_fixtures import field,frequency,packed,video_fixture
ROOT=Path(__file__).resolve().parents[1];FIX=ROOT/'tests/fixtures/playback-errors';OUT=FIX/'aac-ld-mps-sbr-pcm';FRAMES=32
@lru_cache(None)
def core_data(n,cfg,factor):
 files=core(True,specs=[(n,24000,cfg,'base',0,0,False)],output_samples=n//factor,synthesis_window=short_window if factor!=1 else None,frame_count=FRAMES);c=json.loads(files['manifest.json'])['cases'][0];return c,struct.unpack('<'+'f'*(len(files[c['name']+'.f32le'])//4),files[c['name']+'.f32le'])
def main():
 OUT.mkdir(exist_ok=True);am=json.loads((FIX/'aac-ld-mps-assemble/manifest.json').read_text())['cases'];cfgs=(FIX/'aac-ld-mps-assemble/configs.bin').read_bytes();noise_wire=(FIX/'aac-sbr-noise-protocol.f64le').read_bytes();noise=[complex(*struct.unpack_from('<dd',noise_wire,i*16))for i in range(512)];blob=bytearray();cases=[];controls=[];errors=[]
 def rows(frames):
  out=[]
  for bits in frames:
   wire=packed(bits);out.append(dict(offset=len(blob),bytes=len(wire),core_bits=len(bits)));blob.extend(wire)
  return out
 for n in [480,512]:
  for factor in [2,4,8]:
   for double in [False,True]:
    rate=24000//factor*(1+double);sn=n//factor*(1+double);group=32//math.gcd(32,sn)
    while group*sn//32<4:group*=2
    slots=group*sn//32;assert FRAMES%group==0
    for cfg in [1,2,6]:
     elements,mapping,_=LAYOUTS[cfg];channels=len(mapping);source,full=core_data(n,cfg,1);_,short=core_data(n,cfg,factor);lfe=mapping[sum(2 if e==1 else 1 for e in elements[:elements.index(3)])]if 3 in elements else None
     for mode in (['base','coupled-pan']if cfg==2 else['base']):
      for protected in ([True,False]if n==512 and factor==2 and double and mode=='base'else[True]):
       planar=[]
       for ch in range(channels):
        use_full=ch==lfe or 10>32//factor;pcm=full[ch::channels]if use_full else[v*factor for v in short[ch::channels]];low=[[v*32768 for v in row]for row in analysis(pcm,32 if use_full else 32//factor)];qmf=[r+[0j]*32 for r in low]if ch==lfe else reconstruct([r[:10]for r in low],n//32,noise,mode,ch);planar.append([v/32768 for v in synthesis(qmf,(64 if double else 32)//factor)])
       gold=b''.join(struct.pack('<'+'f'*channels,*(p[t]for p in planar))for t in range(len(planar[0])));prefix=field(31,5)+field(7,6)+frequency(24000)+field(cfg,4)+field(n==480,1)+'000'+'1'+field(double,1)+field(protected,1)+header(0,True)*sum(e!=3 for e in elements)+extension(24000//factor,True);asc=packed(prefix+'0000'+'00');_,hb,lb,_=tables(10,27,0,False,0,0);frames=[r['bits']+''.join(sbr_bits(2 if e==1 else 1,len(hb)-1,len(lb)-1,n//32,i,protected,mode)for e in elements if e!=3)for i,r in enumerate(source['frames'])];name=f'n{n}-ds{factor}-x{1+double}-c{cfg}-{mode}-crc{int(protected)}';base=dict(name=name,n=n,profile=39,factor=factor,configuration=cfg,channels=channels,input_channels=channels,rate=rate,source_rate=24000,container_rate=rate,container_frame_samples=sn,samples=FRAMES*sn,pcm_offset=0,asc=asc.hex(),frames=rows(frames),out_n=sn,slots=slots,bands=32,video=name+'-control.mp4',golden=name+'-control.f32le',double=double,crc=protected,mode=mode);(OUT/base['video']).write_bytes(video_fixture([base],blob,channels,write=False)['data']);(OUT/base['golden']).write_bytes(gold);controls.append(base)
       for tree in ([2,3,4]if cfg==2 else[0,1,7]if cfg==1 else[5,6]):
        c=next(c for c in am if c['tree']==tree and c['variant']==1 and c['bands']==23);ssc=config(c,48000,slots,cfgs[c['config_offset']:c['config_offset']+c['config_bytes']],0);ssc=packed(frequency(rate)+raw(ssc)[4:]);spatial_asc=packed(prefix+field(2,4)+length(len(ssc))+raw(ssc)+'0000'+'00');suffix=['']*FRAMES;reference=[]
        for i in range(FRAMES//group):
         payload,_=body(c,i,slots,'alternating',False);suffix[(i+1)*group-1]=envelope(9,1 if i==0 else 0,3,(bytes([len(ssc)])+ssc if i==0 else b'')+payload,True);reference.append(payload.hex())
        packets=[bits+tail for bits,tail in zip(frames,suffix)];stem=name+f'-tree{tree}';outputs=[6,6,6,8,8,8,8,2][tree];case=dict(base,name=stem,asc=spatial_asc.hex(),frames=rows(packets),tree=tree,channels=outputs,group=group,ssc=ssc.hex(),reference=reference,core_gold=base['golden'],video=stem+'.mp4',file=stem+'.mkv',live_prefix_packets=group+2,edges=[group*sn,2*group*sn,3*group*sn]);(OUT/case['video']).write_bytes(video_fixture([case],blob,outputs,write=False)['data']);(OUT/case['file']).write_bytes(mkv(spatial_asc,[packed(b)for b in packets],rate,sn,outputs));cases.append(case)
        if n==512 and factor in[2,8]and not double and cfg==1 and mode=='base'and protected and tree==0:
         bad_bits=packets.copy();bit=len(source['frames'][3]['bits']);v=bad_bits[3];bad_bits[3]=v[:bit]+('1'if v[bit]=='0'else'0')+v[bit+1:];bad=dict(case,name=stem+'-bad-crc',video=stem+'-bad-crc.mp4',frames=rows(bad_bits),control=case['name'],bad_index=3,error='low-delay SBR CRC mismatch');(OUT/bad['video']).write_bytes(video_fixture([bad],blob,outputs,write=False)['data']);errors.append(bad)
 # Saved independently authored wide-LF cases exercise nominal companions and
 # changing coded crossover above the adapted analysis width.
 wide=FIX/'aac-eld-crossover-wide';wm=json.loads((wide/'manifest.json').read_text());wb=(wide/'packets.bin').read_bytes()
 for parent in wm['cases']:
  if not(parent['name'].startswith('n512-c1-start1-')and parent['factor']==4):continue
  double='-x2-'in parent['name'];n=512;sn=parent['out_n'];rate=parent['rate'];slots=sn//32;group=1;original=raw(bytes.fromhex(parent['asc']));ext=extension(6000,True);at=original.rfind(ext+'000000');assert at>=0;head=original[:at];assert packed(head+ext+'000000').hex()==parent['asc'];assert eld_prefix(packed(head+'000000').hex())[0]==head
  frames=[raw(wb[r['offset']:r['offset']+r['bytes']])[:r['end_bits']]for r in parent['frames']];name='wide-'+parent['name'];base=dict(parent,name=name,profile=39,source_rate=24000,input_channels=1,configuration=1,asc=parent['asc'],frames=rows(frames),video=name+'-control.mp4',golden=name+'-control.f32le',double=double,crc=True,mode='wide-lf');(OUT/base['video']).write_bytes(video_fixture([base],blob,1,write=False)['data']);(OUT/base['golden']).write_bytes((wide/parent['golden']).read_bytes());controls.append(base)
  for tree in [0,1,7]:
   c=next(c for c in am if c['tree']==tree and c['variant']==1 and c['bands']==23);ssc=config(c,48000,slots,cfgs[c['config_offset']:c['config_offset']+c['config_bytes']],0);ssc=packed(frequency(rate)+raw(ssc)[4:]);asc=packed(head+ext+field(2,4)+length(len(ssc))+raw(ssc)+'000000');reference=[];packets=[]
   for i,bits in enumerate(frames):
    payload,_=body(c,i,slots,'alternating',False);reference.append(payload.hex());packets.append(bits+envelope(9,1 if i==0 else 0,3,(bytes([len(ssc)])+ssc if i==0 else b'')+payload,True))
   stem=name+f'-tree{tree}';outputs=[6,6,6,8,8,8,8,2][tree];case=dict(base,name=stem,asc=asc.hex(),frames=rows(packets),tree=tree,channels=outputs,group=1,ssc=ssc.hex(),reference=reference,core_gold=base['golden'],video=stem+'.mp4',file=stem+'.mkv',live_prefix_packets=3,edges=[sn,2*sn,3*sn]);(OUT/case['video']).write_bytes(video_fixture([case],blob,outputs,write=False)['data']);(OUT/case['file']).write_bytes(mkv(asc,[packed(b)for b in packets],rate,sn,outputs));cases.append(case)
 (OUT/'packets.bin').write_bytes(blob);(OUT/'manifest.json').write_text(json.dumps(dict(cases=cases,controls=controls,errors=errors,exports=[dict(c,video=c['file'],name=c['name']+'-matroska')for c in cases],scope='Own downscaled LD-SBR with explicit adapted SSC,through CLDFB PCM then ordinary MPS PCM interface;not downscaled shared QMF or nominal-SSC remapping. Independent direct core/CLDFB/SBR PCM plus qualified spatial composition,not whole-codec conformance.'),separators=(',',':'))+'\n');print(len(cases),'spatial programmes;',len(controls),'independent LD-SBR PCM controls')
if __name__=='__main__':main()
