#!/usr/bin/env python3
"""Own spatial frames cutting across AAC access units; explicit offline generation."""
import json,shutil
from pathlib import Path
from generate_aac_eld_fixtures import mkv
from generate_aac_ld_mps_inband_fixtures import raw
from generate_aac_ld_mps_transport_fixtures import envelope,length
from generate_he_aac_packet_fixtures import packed,field,video_fixture
ROOT=Path(__file__).resolve().parents[1];FIX=ROOT/'tests/fixtures/playback-errors'
PARENT=FIX/'aac-ld-mps-frame-length';OUT=FIX/'aac-ld-mps-source-slices';FRAMES=12

def main():
 OUT.mkdir(exist_ok=True);parent=json.loads((PARENT/'manifest.json').read_text());data=(PARENT/'packets.bin').read_bytes();blob=bytearray();cases=[];controls=[];inputs={};errors=[]
 def rows(values):
  result=[]
  for bits in values:
   wire=packed(bits);result.append(dict(offset=len(blob),bytes=len(wire),bits=len(bits)));blob.extend(wire)
  return result
 for base in parent['fixed']:
  control={k:base[k]for k in('name','video','rate','samples','channels')};shutil.copyfile(PARENT/base['video'],OUT/base['video']);controls.append(control)
  prefixes=[raw(data[r['offset']:r['offset']+r['bytes']])[:r['prefix_bits']]for r in base['frames']]
  source=base['name'].rsplit('-tree',1)[0];shared=base.get('shared',False);bins=base['qmf_bins'];core=base['out_n'];slots=base['slots'];group=base['group']
  if source not in inputs:
   if shared:
    file=source+'-input.qmf64le';count=base['input_channels']*FRAMES*(base['n']//32)*base['expected_bands']*16;offset=base['expected_offset'];gold=(FIX/'aac-eld-spatial-sbr/expected.f64le').read_bytes();(OUT/file).write_bytes(gold[offset:offset+count])
   else:
    file=base['golden'];shutil.copyfile(PARENT/file,OUT/file)
   inputs[source]=file
  body=bytes.fromhex(base['schedule'][0]['body']);initial=bytes.fromhex(base['ssc'])
  def ssc(n):return packed(raw(initial)[:4]+field(n-1,5)+raw(initial)[9:])
  for pattern in('staggered','batch','initial','straddling'):
   plan=[];start=0
   if pattern=='straddling'or pattern=='initial'and base['tree']%2:
    first=slots if pattern=='straddling'else min(32,2*slots)
    widths=[first];remaining=FRAMES//group*slots-first;step=0
    while remaining:
     width=min(remaining,min(32,slots+3 if step%2==0 else 2*slots-1))
     if remaining-width==1:width-=1
     assert width>=2
     widths.append(width);remaining-=width;step+=1
   else:
    widths=[]
    for index in range(FRAMES//group):
     a=slots//2-(index%2)
     widths.extend([slots]if index==0 and pattern!='initial'else[a,slots-a])
   for index,n in enumerate(widths):
    stop=start+n*bins;arrival=start//core
    if pattern=='batch':arrival=FRAMES-1
    elif pattern=='staggered'and index>0 and index%2==0:arrival=min(FRAMES-1,(stop+core-1)//core)
    if plan:arrival=max(arrival,plan[-1]['arrival'])
    plan.append(dict(start=start,end=stop,slots=n,arrival=arrival,ssc=ssc(n).hex(),body=body.hex()));start=stop
   assert start==FRAMES*core and all(0<=r['arrival']<FRAMES for r in plan)
   first=bytes.fromhex(plan[0]['ssc']);asc=raw(bytes.fromhex(base['asc']))
   marker=field(0x7cc,11)+'1'+field(len(initial),8)+raw(initial)if base.get('profile')==23 else field(2,4)+length(len(initial))+raw(initial)
   at=asc.rfind(marker);assert at>=0
   replacement=marker[:-len(raw(initial))]+raw(first);asc=packed(asc[:at]+replacement+asc[at+len(marker):])
   messages={}
   for i,r in enumerate(plan):
    config=bytes.fromhex(r['ssc']);payload=body if i==0 else bytes([len(config)])+config+body
    messages.setdefault(r['arrival'],[]).append(envelope(9,0 if i==0 else 1,3,payload,True))
   name=base['name'].removesuffix('-fixed')+'-'+pattern
   frames=rows([p+''.join(messages.get(i,[]))for i,p in enumerate(prefixes)])
   for row,prefix in zip(frames,prefixes):row['prefix_bits']=len(prefix)
   case={k:base[k]for k in('n','out_n','rate','samples','channels','input_channels','tree','qmf_bins','group','slots')}
   case.update(bands=bins,name=name,pattern=pattern,shared=shared,asc=asc.hex(),ssc=first.hex(),frames=frames,plan=plan,control=base['video'],input=inputs[source],stored_bands=base.get('expected_bands',0),video=name+'.mp4',file=name+'.mkv',container_rate=base['rate'],container_frame_samples=core,pcm_offset=0,live_prefix_packets=3,edges=[r['end']for r in plan[:3]])
   (OUT/case['video']).write_bytes(video_fixture([case],blob,case['channels'],write=False)['data'])
   (OUT/case['file']).write_bytes(mkv(asc,[packed(raw(blob[r['offset']:r['offset']+r['bytes']])[:r['bits']])for r in frames],case['rate'],core,case['channels']));cases.append(case)
 for good in [c for c in cases if c['name']in('aac-ld-mps-downscaled-aot23-n480-ds2-c1-tree7-batch','aac-ld-mps-sbr-rates-n512-r48000-c2-x2-tree2-batch','aac-eld-spatial-sbr-n512-c1-base-x1-crc1-tree7-staggered','aac-eld-spatial-sbr-n480-c2-base-x2-crc1-tree2-staggered')]:
  target=11 if good['pattern']=='batch'else good['plan'][2]['arrival']
  broken=max(i for i,r in enumerate(good['plan'])if r['arrival']==target)
  messages={}
  for i,r in enumerate(good['plan']):
   config=bytes.fromhex(r['ssc']);body=bytes.fromhex(r['body'])+(b'\xff'if i==broken else b'')
   payload=body if i==0 else bytes([len(config)])+config+body
   messages.setdefault(r['arrival'],[]).append(envelope(9,0 if i==0 else 1,3,payload,True))
  prefixes=[raw(blob[r['offset']:r['offset']+r['bytes']])[:r['prefix_bits']]for r in good['frames']]
  frames=rows([p+''.join(messages.get(i,[]))for i,p in enumerate(prefixes)])
  bad=dict(good,name=good['name']+'-bad-trailing',video=good['name']+'-bad-trailing.mp4',frames=frames,good_video=good['video'],bad_packet=target,error='trailing bytes after LD-MPS spatial frame')
  (OUT/bad['video']).write_bytes(video_fixture([bad],blob,bad['channels'],write=False)['data']);errors.append(bad)
 (OUT/'packets.bin').write_bytes(blob)
 (OUT/'manifest.json').write_text(json.dumps(dict(cases=cases,controls=controls,errors=errors,exports=[dict(c,video=c['file'],name=c['name']+'-matroska')for c in cases],scope='Own complete spatial-frame partitions crossing AAC source windows,including partial initial SSC,staggered and batched messages. Fixed-partition controls isolate input and partition invariance after common warm-up. Saved analytical PCM or independent spatial-bank SBR QMF supplies direct component composition. No private media or production FFmpeg.'),separators=(',',':'))+'\n')
 print(len(cases),'split-source programmes;',len(controls),'fixed controls;',len(inputs),'input references')
if __name__=='__main__':main()
