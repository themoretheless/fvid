#!/usr/bin/env python3
"""Authored SSC frame-length transitions; explicit offline generation only."""
import json, math
from pathlib import Path
from generate_aac_eld_fixtures import mkv
from generate_aac_eld_downscale_info_fixtures import extension, prefix as asc_prefix
from generate_aac_ld_mps_stp_fixtures import config
from generate_aac_ld_mps_entropy_fixtures import pcm
from generate_aac_ld_mps_inband_fixtures import raw
from generate_aac_ld_mps_transport_fixtures import envelope, length
from generate_he_aac_packet_fixtures import packed, field, frequency, video_fixture
ROOT=Path(__file__).resolve().parents[1]
FIX=ROOT/'tests/fixtures/playback-errors'
OUT=FIX/'aac-ld-mps-frame-length'
FRAMES=12

def main():
 OUT.mkdir(exist_ok=True)
 models=json.loads((FIX/'aac-ld-mps-assemble/manifest.json').read_text())['cases']
 configs=(FIX/'aac-ld-mps-assemble/configs.bin').read_bytes()
 controls=[];fixed=[];cases=[];errors=[];blob=bytearray()
 def rows(bits):
  result=[]
  for bits in bits:
   wire=packed(bits);result.append(dict(offset=len(blob),bytes=len(wire),core_bits=len(bits)));blob.extend(wire)
  return result
 selections=[]
 for n in (480,512):
  for ch in (1,2,6):
   selections += [('aac-ld-mps-downscaled',f'aot{aot}-n{n}-ds2-c{ch}')for aot in (23,39)]
   selections += [('aac-ld-mps-sbr-pcm',f'n{n}-ds2-x1-c{ch}-base-crc1')]
   selections += [('aac-ld-mps-sbr-rates',f'n{n}-r48000-c{ch}-x{x}')for x in (1,2)]
   selections += [('aac-eld-spatial-sbr',f'n{n}-c{ch}-base-x{x}-crc1')for x in (1,2)]
 for corpus,name in selections:
  parent=FIX/corpus; shared=corpus=='aac-eld-spatial-sbr'
  source=next(c for c in json.loads((parent/'manifest.json').read_text())['cases'if shared else'controls']if c['name']==name)
  data=(parent/'packets.bin').read_bytes()
  prefixes=[raw(data[r['offset']:r['offset']+r['bytes']])[:r['core_bits']+(r['sbr_bits']if shared else 0)]for r in source['frames'][:FRAMES]]
  wire=raw(bytes.fromhex(source['asc']))
  is_ld=source.get('profile')==23
  if shared:
   tree={1:7,2:2,6:5}[source['channels']]
   model=next(c for c in models if c['tree']==tree and c['variant']==1 and c['bands']==23)
   old=configs[model['config_offset']:model['config_offset']+model['config_bytes']]
   old=packed(frequency(source['rate'])+field(source['slots']-1,5)+raw(old)[9:])
   marker=field(2,4)+length(len(old))+raw(old)+'000000';at=wire.rfind(marker);assert at>=0
   head=wire[:at];source=dict(source,factor=1,out_n=source['n']*(2 if source['bands']==64 else 1),asc=packed(head+'000000').hex(),shared=True)
  elif is_ld:head=wire[:18]
  elif source['factor']!=1:
   ext=extension(source['rate']//(2 if source.get('double',False)else 1),True);at=wire.rfind(ext+'000000');assert at>=0;head=wire[:at]+ext
  else:head,_=asc_prefix(source['asc'])
  if not is_ld:assert packed(head+'000000').hex()==source['asc']
  stem=corpus+'-'+name
  base=dict(source,name=stem,frames=rows(prefixes),samples=FRAMES*source['out_n'],video=stem+'-bare.mp4',golden=stem+'-bare.f32le')
  if not shared:
   (OUT/base['video']).write_bytes(video_fixture([base],blob,source['channels'],write=False)['data'])
   (OUT/base['golden']).write_bytes((parent/source['golden']).read_bytes()[:FRAMES*source['out_n']*source['channels']*4]);controls.append(base)
  bins=32 if source['rate']<27713 else 128 if source['rate']>=55426 else 64
  group=bins//math.gcd(bins,source['out_n']);slots=group*source['out_n']//bins
  assert group in(1,2)and slots in(8,15,16)
  groups=([1,4,1,2,1,2,1]if slots==8 else[1,2,1,2,1,2,1,1,1])if group==1 else[1,2,1,1,1]
  assert sum(groups)*group==FRAMES and max(groups)*slots<=32
  trees={1:[0,1,7],2:[2,3,4],6:[5,6]}[source['channels']]
  for tree in trees:
   model=next(c for c in models if c['tree']==tree and c['variant']==1 and c['bands']==23)
   original=config(model,48000,slots,configs[model['config_offset']:model['config_offset']+model['config_bytes']],0)
   ssc=packed(frequency(source['rate'])+raw(original)[4:])
   asc=packed(head+field(0x7cc,11)+'1'+field(len(ssc),8)+raw(ssc))if is_ld else packed(head+field(2,4)+length(len(ssc))+raw(ssc)+'000000')
   # Identical absolute-coded endpoints and no smoothing. After the common
   # warm-up frame, changing only the frame partition must preserve the PCM.
   bits='011'
   for r in model['frames'][0]['parameters']:
    offset,levels={'CLD':(15,31),'ICC':(0,8),'CPC':(20,51)}[r['kind']]
    bits+='1111'+'10001'+pcm([v+offset for v in r['values'][0]for _ in range(2)],levels)
   body=packed(bits+'0000')
   def movie(pattern,groups):
    schedule=[];start=0;messages={}
    for index,mult in enumerate(groups):
     end=start+mult*group;current=packed(raw(ssc)[:4]+field(slots*mult-1,5)+raw(ssc)[9:])
     header=bytes([len(current)])+current+body
     arrival=start if pattern=='early'else end-1 if pattern in('late','fixed')else FRAMES-1
     messages.setdefault(arrival,[]).append(envelope(9,0 if index==0 else 1,3,body if index==0 else header,True))
     schedule.append(dict(start=start,end=end,slots=slots*mult,ssc=current.hex(),body=body.hex(),arrival=arrival));start=end
    assert start==FRAMES
    frames=rows([p+''.join(messages.get(i,[]))for i,p in enumerate(prefixes)])
    for row,p in zip(frames,prefixes):row['prefix_bits']=len(p)
    case_name=f'{stem}-tree{tree}-{pattern}'
    case=dict(base,name=case_name,asc=asc.hex(),frames=frames,tree=tree,channels=6 if tree<=2 else 2 if tree==7 else 8,input_channels=source['channels'],qmf_bins=bins,group=group,slots=slots,ssc=ssc.hex(),schedule=schedule,video=case_name+'.mp4',file=case_name+'.mkv',live_prefix_packets=3,edges=[r['start']*source['out_n']for r in schedule[1:4]])
    (OUT/case['video']).write_bytes(video_fixture([case],blob,case['channels'],write=False)['data'])
    (OUT/case['file']).write_bytes(mkv(asc,[packed(raw(blob[r['offset']:r['offset']+r['bytes']])[:r['core_bits']])for r in frames],source['rate'],source['out_n'],case['channels']))
    return case
   control=movie('fixed',[1]*(FRAMES//group));fixed.append(control)
   for pattern in ('early','late','batch'):
    cases.append(dict(movie(pattern,groups),control=control['video']))
 # Late failure after earlier changed-length frames have already rendered
 # inside the same transactional submit. Also isolate the whole-core boundary.
 for good in [c for c in cases if c['name']in('aac-ld-mps-downscaled-aot23-n480-ds2-c1-tree7-batch','aac-ld-mps-sbr-rates-n512-r48000-c2-x2-tree2-batch')]:
  for failure in ('trailing','partial-core'):
   messages={}
   for i,entry in enumerate(good['schedule']):
    current=bytes.fromhex(entry['ssc']);body=bytes.fromhex(entry['body'])
    if i==len(good['schedule'])-1:
     if failure=='trailing':body+=b'\xff'
     else:current=packed(raw(current)[:4]+field(entry['slots']-2,5)+raw(current)[9:])
    messages.setdefault(entry['arrival'],[]).append(envelope(9,0 if i==0 else 1,3,body if i==0 else bytes([len(current)])+current+body,True))
   prefixes=[raw(blob[r['offset']:r['offset']+r['bytes']])[:r['prefix_bits']]for r in good['frames']]
   frames=rows([p+''.join(messages.get(i,[]))for i,p in enumerate(prefixes)])
   name=good['name']+'-bad-'+failure
   bad=dict(good,name=name,frames=frames,video=name+'.mp4',good_video=good['video'],error='trailing bytes after LD-MPS spatial frame'if failure=='trailing'else'in-band frame length splits an AAC core packet')
   (OUT/bad['video']).write_bytes(video_fixture([bad],blob,bad['channels'],write=False)['data']);errors.append(bad)
 (OUT/'packets.bin').write_bytes(blob)
 (OUT/'manifest.json').write_text(json.dumps(dict(controls=controls,fixed=fixed,cases=cases,errors=errors,exports=[dict(c,video=c['file'],name=c['name']+'-matroska')for c in cases],scope='Authored same-format SSC frame-length switches,32/64/128-bin PCM ingress,AAC-LD/ELD480/512 and LD-SBR;whole-core packet grouping. Early,late and batched headers. Fixed-partition independently decodable controls have the same constant spatial parameters and waveform. No private media,no FFmpeg.'),separators=(',',':'))+'\n')
 print(len(cases),'transition programmes;',len(fixed),'fixed-partition controls;',len(controls),'bare controls')
if __name__=='__main__':main()
