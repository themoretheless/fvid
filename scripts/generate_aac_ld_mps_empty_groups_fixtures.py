#!/usr/bin/env python3
"""Owned zero-width OTT grouped payloads; explicit offline generation."""
import json
from pathlib import Path
from generate_aac_eld_fixtures import mkv
from generate_aac_ld_mps_stp_fixtures import config
from generate_aac_ld_mps_inband_fixtures import raw
from generate_aac_ld_mps_transport_fixtures import envelope,length
from generate_aac_ld_mps_entropy_fixtures import Writer,quant,pcm
from generate_he_aac_packet_fixtures import field,packed,frequency,video_fixture
ROOT=Path(__file__).resolve().parents[1];FIX=ROOT/'tests/fixtures/playback-errors';PARENT=FIX/'aac-ld-mps-frame-length';OUT=FIX/'aac-ld-mps-empty-groups'
BOOKS=json.loads((FIX/'aac-ld-mps-entropy/codewords.json').read_text())

def entropy(kind,coarse,paired,independent,set_index,scheme,seed):
 levels,offset,lsb=quant(kind,coarse);values=[(seed+i*2)%7-3 for i in range(1+paired)]
 if kind=='ICC':values=[(seed+i)%min(levels,3)for i in range(1+paired)]
 allow=not independent or set_index!=0;time=[bool(seed%2),bool((seed+1)%2)]
 if not paired and not allow:time[0]=False
 if paired and time[0]and not allow:time[1]=False
 w=Writer(BOOKS);w.wire='1'if scheme=='pcm'else'0'
 if scheme=='pcm':w.wire+=pcm([v+offset for v in values],levels)
 else:
  if paired or allow:w.wire+=field(time[0],1)
  if paired and(not time[0]or allow):w.wire+=field(time[1],1)
  w.wire+=field(scheme=='2d',1);msb=[(v+offset)>>lsb for v in values]
  for row in range(1+paired):
   previous=(offset>>lsb)if row==0 else msb[0]
   delta=msb[row]if not time[row]or row==0 and not allow else msb[row]-previous
   w.vector(kind,time[row],[delta],seed%4 if scheme=='2d'else None,False)
  if lsb:w.wire+=''.join(field((v+offset)&1,1)for v in values)
 return w.wire,values

def block(kind,coarse,paired,independent,scheme,seed,stride_code,sets=2):
 bits='11'*sets;wanted=[]
 for ps in range(1 if paired else sets):
  payload,values=entropy(kind,coarse,paired,independent,ps,scheme,seed+ps)
  bits+=field(paired,1)+field(coarse,1)+field(stride_code,2)+payload;wanted.extend(values)
 return bits,wanted

def main():
 OUT.mkdir(exist_ok=True);parent=json.loads((PARENT/'manifest.json').read_text());data=(PARENT/'packets.bin').read_bytes();models=json.loads((FIX/'aac-ld-mps-assemble/manifest.json').read_text())['cases'];configs=(FIX/'aac-ld-mps-assemble/configs.bin').read_bytes();blob=bytearray();controls=[];cases=[];units=[];errors=[]
 def rows(values):
  result=[]
  for bits in values:
   wire=packed(bits);result.append(dict(offset=len(blob),bytes=len(wire),bits=len(bits)));blob.extend(wire)
  return result
 for base in [c for c in parent['fixed']if c['tree']in(0,1)]:
  tree=base['tree'];slots=base['slots'];model=next(c for c in models if c['tree']==tree and c['variant']==0 and c['bands']==23)
  original=config(model,48000,slots,configs[model['config_offset']:model['config_offset']+model['config_bytes']],0);ssc=packed(frequency(base['rate'])+raw(original)[4:]);old=bytes.fromhex(base['ssc']);asc=raw(bytes.fromhex(base['asc']))
  marker=field(0x7cc,11)+'1'+field(len(old),8)+raw(old)if base.get('profile')==23 else field(2,4)+length(len(old))+raw(old);at=asc.rfind(marker);assert at>=0
  replacement=(field(0x7cc,11)+'1'+field(len(ssc),8)if base.get('profile')==23 else field(2,4)+length(len(ssc)))+raw(ssc);asc=packed(asc[:at]+replacement+asc[at+len(marker):])
  prefixes=[raw(data[r['offset']:r['offset']+r['bytes']])[:r['prefix_bits']]for r in base['frames']];regions=model['frames'][0]['parameters'];assert sum(not r['values'][0]for r in regions)==1
  def movie(stride,scheme):
   suffix=['']*12;reference=[]
   for i in range(12//base['group']):
    independent=i%4==0;bits='01'+field(independent,1);zero=[]
    for r in regions:
     if r['values'][0]:
      offset,levels={'CLD':(15,31),'ICC':(0,8),'CPC':(20,51)}[r['kind']];bits+='1111'+'1000'+'1'+pcm([v+offset for v in r['values'][0]for _ in range(2)],levels)
     elif stride==0:bits+='1111'+'1000'+'1'
     else:
      wire,wanted=block('CLD',bool(i%3),i%2==0,independent,scheme,i,stride);zero.append(dict(offset=len(bits),bits=len(wire),wire=packed(wire).hex(),coarse=bool(i%3),paired=i%2==0,independent=independent,values=wanted));bits+=wire
    bits+='0000';body=packed(bits);reference.append(dict(body=body.hex(),bits=len(bits),empty=zero,independent=independent));suffix[(i+1)*base['group']-1]=envelope(9,0,3,body,True)
   name=base['name'].removesuffix('-fixed')+f'-empty-s{stride}-{scheme}';frames=rows([p+s for p,s in zip(prefixes,suffix)]);case={k:base[k]for k in('n','out_n','rate','samples','channels','input_channels','tree','qmf_bins','group','slots')};case.update(name=name,asc=asc.hex(),ssc=ssc.hex(),bands=base['qmf_bins'],frames=frames,reference=reference,stride_code=stride,scheme=scheme,video=name+'.mp4',file=name+'.mkv',container_rate=base['rate'],container_frame_samples=base['out_n'],pcm_offset=0,live_prefix_packets=3,edges=[base['out_n']*base['group'],3*base['out_n']*base['group']])
   (OUT/case['video']).write_bytes(video_fixture([case],blob,case['channels'],write=False)['data']);(OUT/case['file']).write_bytes(mkv(asc,[packed(raw(blob[r['offset']:r['offset']+r['bytes']])[:r['bits']])for r in frames],case['rate'],case['out_n'],case['channels']));return case
  control=movie(0,'pcm');controls.append(control)
  for stride in(1,2,3):
   for scheme in('pcm','1d','2d'):cases.append(dict(movie(stride,scheme),control=control['video']))
 # Corrupt a late empty-region symbol, preserving all envelope CRCs and
 # preceding nonzero audio. This must fail on the unused PCM value itself.
 for good in [c for c in cases if c['stride_code']==1 and c['scheme']=='pcm']:
  reference=good['reference'];index=3;record=reference[index];zero=record['empty'][0]
  assert not zero['coarse'] and not zero['paired']
  bits=raw(bytes.fromhex(record['body']))[:record['bits']];at=zero['offset']+4+4+1
  # Fine unpaired CLD has 31 levels, so codeword31 is unused.
  bits=bits[:at]+field(31,5)+bits[at+5:]
  packet_index=(index+1)*good['group']-1
  frames=[dict(r)for r in good['frames']];old=frames[packet_index]
  # Recover the source prefix from the corresponding parent input packet.
  base=next(c for c in parent['fixed']if good['name'].startswith(c['name'].removesuffix('-fixed')+'-empty-'))
  original=base['frames'][packet_index];prefix=raw(data[original['offset']:original['offset']+original['bytes']])[:original['prefix_bits']]
  frames[packet_index]=rows([prefix+envelope(9,0,3,packed(bits),True)])[0]
  bad=dict(good,name=good['name']+'-invalid-pcm',video=good['name']+'-invalid-pcm.mp4',frames=frames,good_video=good['video'],bad_packet=packet_index,error='unused LD-MPS PCM codeword')
  (OUT/bad['video']).write_bytes(video_fixture([bad],blob,bad['channels'],write=False)['data']);errors.append(bad)
 # Split TTT low/high regions can share the virtual history cell. Preserve a
 # specific refusal until borrowed-history semantics have independent proof.
 base=next(c for c in parent['fixed']if c['tree']==2 and c['n']==480 and c.get('profile')==39 and c['rate']==48000)
 model=next(c for c in models if c['tree']==2 and c['variant']==1 and c['bands']==23)
 original=config(model,48000,base['slots'],configs[model['config_offset']:model['config_offset']+model['config_bytes']],0)
 s=frequency(base['rate'])+raw(original)[4:];s=s[:47]+'00000'+s[52:];ssc=packed(s)
 old=bytes.fromhex(base['ssc']);asc_bits=raw(bytes.fromhex(base['asc']));marker=field(2,4)+length(len(old))+raw(old);at=asc_bits.rfind(marker);assert at>=0
 asc=packed(asc_bits[:at]+field(2,4)+length(len(ssc))+raw(ssc)+asc_bits[at+len(marker):])
 prefixes=[raw(data[r['offset']:r['offset']+r['bytes']])[:r['prefix_bits']]for r in base['frames']]
 regions=model['frames'][0]['parameters'];ttt_controls=[];refusals=[]
 for refusal in(False,True):
  suffix=['']*12
  for i in range(12//base['group']):
   independent=i%4==0;bits='01'+field(independent,1);first=True
   for r in regions:
    values=r['values'][0];parts=r['id'].split(':')
    if parts[0].startswith('ttt-'):
     if parts[2]=='0':values=[]
     else:
      low=next(v for v in regions if v['id']==':'.join(parts[:2]+['0']+parts[3:]));values=low['values'][0]+values
    if values:
     offset,levels={'CLD':(15,31),'ICC':(0,8),'CPC':(20,51)}[r['kind']];bits+='1111'+'1000'+'1'+pcm([v+offset for v in values for _ in range(2)],levels)
    elif refusal and i==3 and first:
     bits+=block('CLD',False,True,independent,'pcm',0,1)[0];first=False
    else:bits+='1111'+'1000'+'1'
   suffix[(i+1)*base['group']-1]=envelope(9,0,3,packed(bits+'0000'),True)
  name='split-ttt-empty-history-'+('refusal'if refusal else'control');frames=rows([p+s for p,s in zip(prefixes,suffix)]);case={k:base[k]for k in('n','out_n','rate','samples','channels','input_channels','tree','qmf_bins','group','slots')};case.update(name=name,asc=asc.hex(),ssc=ssc.hex(),frames=frames,video=name+'.mp4',pcm_offset=0,container_rate=base['rate'],container_frame_samples=base['out_n'])
  (OUT/case['video']).write_bytes(video_fixture([case],blob,case['channels'],write=False)['data'])
  if refusal:refusals.append(dict(case,good_video=ttt_controls[0]['video'],bad_packet=4*base['group']-1,error='zero-width TTT grouped history is not yet qualified'))
  else:ttt_controls.append(case)
 for kind in('CLD','ICC','CPC'):
  for stride in(1,2,3):
   for coarse in(False,True):
    for paired in(False,True):
     for independent in(False,True):
      for scheme in('pcm','1d','2d'):
       for seed in range(4):
        bits,wanted=block(kind,coarse,paired,independent,scheme,seed,stride);units.append(dict(kind=kind,stride_code=stride,coarse=coarse,paired=paired,independent=independent,scheme=scheme,seed=seed,bits=len(bits),wire=packed(bits).hex(),dummy=wanted,default={'CLD':15,'ICC':0,'CPC':10}[kind]))
 (OUT/'packets.bin').write_bytes(blob);(OUT/'manifest.json').write_text(json.dumps(dict(cases=cases,controls=controls,units=units,errors=errors,ttt_controls=ttt_controls,refusals=refusals,exports=[dict(c,video=c['file'],name=c['name']+'-matroska')for c in cases],scope='Authored empty LFE OTT grouped coding for CLD,paired/unpaired,coarse/fine,PCM/Huffman1D/2D and dependent/independent contexts. Unit CLD/ICC/CPC dummy symbols;owned movies and unchanged zero-band audio controls. No private media,FFmpeg or network.'),separators=(',',':'))+'\n');print(len(cases),'programmes;',len(controls),'controls;',len(units),'parameter units')
if __name__=='__main__':main()
