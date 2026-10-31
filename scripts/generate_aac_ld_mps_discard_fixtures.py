#!/usr/bin/env python3
"""Own empty split-TTT movies and discarded-symbol bit-cursor cases,offline."""
import json
from pathlib import Path
from generate_aac_ld_mps_empty_groups_fixtures import block,Writer,BOOKS,quant
from generate_aac_ld_mps_entropy_fixtures import pcm
from generate_aac_eld_fixtures import mkv
from generate_aac_ld_mps_stp_fixtures import config
from generate_aac_ld_mps_inband_fixtures import raw
from generate_aac_ld_mps_transport_fixtures import envelope,length
from generate_he_aac_packet_fixtures import field,packed,frequency,video_fixture
ROOT=Path(__file__).resolve().parents[1];FIX=ROOT/'tests/fixtures/playback-errors';PARENT=FIX/'aac-ld-mps-frame-length';OUT=FIX/'aac-ld-mps-discard'

def discarded(kind,coarse,paired,independent,scheme,seed,stride):
 if independent or scheme=='pcm':return block(kind,coarse,paired,independent,scheme,seed,stride)[0]
 # The virtual zero would yield an invalid index. An active neighbor's raw
 # history (-6/-3 CLD,-8/-4 CPC,2/1 ICC) yields an in-range discarded index.
 delta={'CLD':8 if coarse else 16,'ICC':-1,'CPC':16}[kind]
 bits='1111'
 for ps in range(1 if paired else 2):
  w=Writer(BOOKS);w.wire='0'+'1'+('1'if paired else'')+field(scheme=='2d',1)
  for row in range(1+paired):w.vector(kind,True,[delta if row==0 else 0],seed%4 if scheme=='2d'else None,False)
  if quant(kind,coarse)[2]:w.wire+='0'*(1+paired)
  bits+=field(paired,1)+field(coarse,1)+field(stride,2)+w.wire
 return bits

def main():
 OUT.mkdir(exist_ok=True);parent=json.loads((PARENT/'manifest.json').read_text());data=(PARENT/'packets.bin').read_bytes();models=json.loads((FIX/'aac-ld-mps-assemble/manifest.json').read_text())['cases'];configs=(FIX/'aac-ld-mps-assemble/configs.bin').read_bytes();blob=bytearray();cases=[];controls=[];units=[]
 def rows(values):
  out=[]
  for bits in values:
   wire=packed(bits);out.append(dict(offset=len(blob),bytes=len(wire),bits=len(bits)));blob.extend(wire)
  return out
 for base in [c for c in parent['fixed']if c['tree']in(2,3,4)]:
  for variant in(0,1):
   model=next(c for c in models if c['tree']==base['tree']and c['variant']==variant and c['bands']==23)
   original=config(model,48000,base['slots'],configs[model['config_offset']:model['config_offset']+model['config_bytes']],0)
   s=frequency(base['rate'])+raw(original)[4:];ssc=packed(s[:47]+'00000'+s[52:]);old=bytes.fromhex(base['ssc']);asc=raw(bytes.fromhex(base['asc']))
   marker=field(0x7cc,11)+'1'+field(len(old),8)+raw(old)if base.get('profile')==23 else field(2,4)+length(len(old))+raw(old);at=asc.rfind(marker);assert at>=0
   replacement=(field(0x7cc,11)+'1'+field(len(ssc),8)if base.get('profile')==23 else field(2,4)+length(len(ssc)))+raw(ssc);asc=packed(asc[:at]+replacement+asc[at+len(marker):])
   prefixes=[raw(data[r['offset']:r['offset']+r['bytes']])[:r['prefix_bits']]for r in base['frames']];regions=model['frames'][0]['parameters']
   def movie(stride,scheme):
    suffix=['']*12;plan=[]
    for i in range(12//base['group']):
     independent=i%4==0;bits='01'+field(independent,1);empty=[]
     for r in regions:
      values=r['values'][0];parts=r['id'].split(':');ttt=parts[0].startswith('ttt-')
      if ttt:
       if parts[2]=='0':values=[]
       else:
        low=next(v for v in regions if v['id']==':'.join(parts[:2]+['0']+parts[3:]));values=low['values'][0]+values
        values=[{'CLD':-6,'CPC':-8,'ICC':2}[r['kind']]]*len(values)
      if values:
       coarse=ttt and bool(i%2);levels,offset,_=quant(r['kind'],coarse)
       if coarse:values=[int(v/2)if r['kind']=='CLD'else v//2 for v in values]
       bits+='1111'+'1'+field(coarse,1)+'00'+'1'+pcm([v+offset for v in values for _ in range(2)],levels)
      elif stride==0:bits+='1111'+'1000'+'1'
      else:
       wire=discarded(r['kind'],bool(i%3),i%2==0,independent,scheme,i,stride);empty.append(dict(kind=r['kind'],wire=packed(wire).hex(),bits=len(wire),offset=len(bits)));bits+=wire
     body=packed(bits+'0000');plan.append(dict(body=body.hex(),empty=empty,independent=independent));suffix[(i+1)*base['group']-1]=envelope(9,0,3,body,True)
    name=base['name'].removesuffix('-fixed')+f'-discard-v{variant}-s{stride}-{scheme}';frames=rows([p+s for p,s in zip(prefixes,suffix)]);c={k:base[k]for k in('n','out_n','rate','samples','channels','input_channels','tree','qmf_bins','group','slots')};c.update(name=name,asc=asc.hex(),ssc=ssc.hex(),frames=frames,plan=plan,variant=variant,stride_code=stride,scheme=scheme,video=name+'.mp4',file=name+'.mkv',container_rate=base['rate'],container_frame_samples=base['out_n'],pcm_offset=0,bands=base['qmf_bins'],live_prefix_packets=3,edges=[base['out_n']*base['group'],3*base['out_n']*base['group']])
    (OUT/c['video']).write_bytes(video_fixture([c],blob,c['channels'],write=False)['data']);(OUT/c['file']).write_bytes(mkv(asc,[packed(raw(blob[r['offset']:r['offset']+r['bytes']])[:r['bits']])for r in frames],c['rate'],c['out_n'],c['channels']));return c
   control=movie(0,'pcm');controls.append(control)
   for stride in(1,2,3):
    for scheme in('pcm','1d','2d'):cases.append(dict(movie(stride,scheme),control=control['video']))
 errors=[]
 for good in [c for c in cases if c['stride_code']==1 and c['scheme']=='pcm']:
  kind='CPC'if good['variant']==0 else'CLD';index=3;record=good['plan'][index];entry=next(v for v in record['empty']if v['kind']==kind)
  bits=raw(bytes.fromhex(record['body']));at=entry['offset']+4+4+1;width=6 if kind=='CPC'else 5;bits=bits[:at]+field((1<<width)-1,width)+bits[at+width:]
  packet_index=(index+1)*good['group']-1;base=next(c for c in parent['fixed']if good['name'].startswith(c['name'].removesuffix('-fixed')+'-discard-'));row=base['frames'][packet_index];prefix=raw(data[row['offset']:row['offset']+row['bytes']])[:row['prefix_bits']]
  frames=[dict(r)for r in good['frames']];frames[packet_index]=rows([prefix+envelope(9,0,3,packed(bits),True)])[0]
  bad={k:v for k,v in good.items()if k!='plan'};bad.update(name=good['name']+'-invalid-pcm',video=good['name']+'-invalid-pcm.mp4',good_video=good['video'],frames=frames,bad_packet=packet_index,error='unused LD-MPS PCM codeword',invalid_kind=kind)
  (OUT/bad['video']).write_bytes(video_fixture([bad],blob,bad['channels'],write=False)['data']);errors.append(bad)
 old=json.loads((FIX/'aac-ld-mps-empty-groups/manifest.json').read_text())
 for i,u in enumerate(old['units']):units.append(dict(u,original=i,adversarial=False))
 for kind in('CLD','ICC','CPC'):
  for coarse in(False,True):
   for paired in(False,True):
    for stride in(1,2,3):
     for scheme in('1d','2d'):
      bits=discarded(kind,coarse,paired,False,scheme,1,stride);units.append(dict(kind=kind,coarse=coarse,paired=paired,independent=False,stride_code=stride,scheme=scheme,bits=len(bits),wire=packed(bits).hex(),default={'CLD':15,'ICC':0,'CPC':10}[kind],adversarial=True))
 (OUT/'packets.bin').write_bytes(blob);(OUT/'manifest.json').write_text(json.dumps(dict(cases=cases,controls=controls,units=units,errors=errors,exports=[dict(c,video=c['file'],name=c['name']+'-matroska')for c in cases],scope='Own split-TTT empty grouped CLD/CPC/ICC movies. Dummy numeric results are unobservable;syntax and bit count remain validated. Source core and video seeds are previously authored fixtures;no private media or FFmpeg.'),separators=(',',':'))+'\n');print(len(cases),'programmes;',len(controls),'controls;',len(units),'units')
if __name__=='__main__':main()
