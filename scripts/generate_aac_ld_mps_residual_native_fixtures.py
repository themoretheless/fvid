#!/usr/bin/env python3
"""Nonconforming LD/residual numeric fixtures. Offline,no external tools."""
import json,re
from pathlib import Path
from generate_aac_ld_mps_transport_fixtures import field,packed,length,envelope
from generate_aac_ld_mps_entropy_fixtures import pcm
from generate_aac_ld_mps_inband_fixtures import raw
from generate_aac_ld_mps_config_fixtures import TREES
from generate_aac_ld_mps_ges_fixtures import config as spatial_config,body as ges_body
from generate_aac_ld_mps_stp_fixtures import body as stp_body
from generate_aac_main_tools_fixtures import ics,channel,sc,tuple_bits
from generate_he_aac_packet_fixtures import video_fixture
ROOT=Path(__file__).resolve().parents[1];FIX=ROOT/'tests/fixtures/playback-errors';OUT=FIX/'aac-ld-mps-residual-native'
SEQUENCES=[0,0,1,2,2,3,0,1,2,3,0,0]
def residual_ics(seq,shape,box,frame,silent,reserved13=False,rate=48000):
 text=(ROOT/'crates/fvid-media/src/owned_aac/aac_band_tables.rs').read_text();kind='SHORT'if seq==2 else'LONG';family={24000:'24K',32000:'48K'if seq==2 else'32K',48000:'48K'}[rate]
 offsets=[int(v)for v in re.findall(r'\d+',re.search(r'const SWB_OFFSET_'+family+'_'+kind+r':[^=]+=\s*\[(.*?)\];',text,re.S)[1])]
 bands=[1,4,len(offsets)-1][box%3];width=3 if seq==2 else 5;escape=(1<<width)-1
 section='';left=bands
 while left>=escape:section+=field(escape,width);left-=escape
 section+=field(left,width)
 wire=field(116+4*(box%3),8)+ics(seq,bands,False,shape=shape)+field(0 if silent else (13 if reserved13 else 1),4)+section
 if not silent:wire+=(field(256,9)+sc(0)*(bands-1))if reserved13 else sc(0)*bands
 wire+='000'
 if not silent and not reserved13:
  for b in range(bands):
   for window in range(8 if seq==2 else 1):
    for at in range(offsets[b],offsets[b+1],4):wire+=tuple_bits([(at+i+box+frame+window)%3-1 for i in range(4)])
 return wire
def payload(c,frame,slots,cutoffs,bad=None,rate=48000):
 bits='';differences=[]
 for box,count in enumerate(cutoffs):
  if not count:differences.append(None);continue
  if box<len(TREES[c['tree']][2]):
   base=next(r for r in c['frames'][0]['parameters']if r['id']==('ott-icc:all'if c['one_icc']else f'ott-icc:{box}'))
   rows=[]
   for ps in range(2):
    bits+='1';row=[]
    for b in range(count):
     value=min(1+(box+frame+ps+b)%2,7-base['values'][ps][b])
     if bad=='icc'and box==next(i for i,v in enumerate(cutoffs)if v)and ps==0 and b==0:value=7
     bits+='0'*value+'1'if value<7 else'00000001';row.append(value)
    rows.append(row)
   differences.append(rows)
  else:differences.append(None)
  seq=SEQUENCES[frame];silent=frame==6
  bits+=residual_ics(seq,(box+frame)%2,box,frame,silent,reserved13=bad=='reserved13',rate=rate)
  if seq==2 and slots in(18,24,30):bits+=residual_ics(seq,(box+frame+1)%2,box,frame+1,silent,reserved13=bad=='reserved13',rate=rate)
 data=packed(bits)if bits else b''
 if bad=='short':data=data[:-1]
 if bad=='fill':
  if len(bits)%8:data=data[:-1]+bytes([data[-1]|((1<<(8-len(bits)%8))-1)])
  data+=b'\xff\x55'
 return data,differences

def control_body(c,differences,temporal_bits=''):
 rows=[]
 for r in c['frames'][0]['parameters']:
  if r['id']=='ott-icc:all':
   for box,lfe in enumerate(TREES[c['tree']][2]):
    if not lfe:rows.append(dict(r,id=f'ott-icc:{box}'))
  else:rows.append(r)
 wire='011'
 for r in rows:
  values=[v[:]for v in r['values']]
  if r['id'].startswith('ott-icc:'):
   box=int(r['id'].split(':')[1])
   if differences[box]:
    for ps,diff in enumerate(differences[box]):
     for b,v in enumerate(diff):values[ps][b]+=v
  levels,offset={'CLD':(31,15),'ICC':(8,0),'CPC':(51,20)}[r['kind']]
  wire+='1111'+'10001'+pcm([values[ps][b]+offset for b in range(len(values[0]))for ps in range(2)],levels)
 return packed(wire+'00'+'1001'+temporal_bits)+b'\0'

def main():
 OUT.mkdir(exist_ok=True);parent=FIX/'aac-ld-mps-stp';catalog=json.loads((parent/'manifest.json').read_text())['cases'];pb=(parent/'packets.bin').read_bytes();assembly=FIX/'aac-ld-mps-assemble';am=json.loads((assembly/'manifest.json').read_text())['cases'];configs=(assembly/'configs.bin').read_bytes();book=json.loads((FIX/'aac-ld-mps-frame/reshape-codewords.json').read_text());blob=bytearray();cases=[];errors=[];refusals=[]
 bases=[c for c in catalog if c['bands']==64 and c['qmf']is not None and c['mode']=='on'and c['parameter_bands']==23]
 def build(base,c,mode,bad=None):
  data=configs[c['config_offset']:c['config_offset']+c['config_bytes']];shape={'stp':1,'ges':2}.get(mode,0);ordinary=spatial_config(c,base['rate'],base['slots'],data,0);ssc=spatial_config(c,base['rate'],base['slots'],data,shape);shape_ssc=ssc
  cutoffs=[0 if lfe else [1,max(1,c['bands']//2),c['bands']][i%3]for i,lfe in enumerate(TREES[c['tree']][2])]+([max(1,c['bands']//2)]if TREES[c['tree']][3]else[])
  cfg=packed(field(3 if base['bands']==64 else 6,4)+'00'+''.join(('1'+field(v,5))if v else'0'for v in cutoffs));extension=packed('0000'+length(len(cfg))+raw(cfg));ssc+=extension
  control=ordinary+extension
  if c['tree']!=7:control=packed(raw(control)[:18]+'0'+raw(control)[19:])
  original=bytes.fromhex(base['ssc']);asc_bits=raw(bytes.fromhex(base['asc']));suffix='0010'+length(len(original))+raw(original)+'0000'+'00';at=asc_bits.rfind(suffix);assert at>=0;asc=packed(asc_bits[:at]+'0010'+length(len(ssc))+raw(ssc)+'0000'+'00')
  payloads=[];reference=[];previous=None
  for i in range(12):
   if shape==2:body,targets=ges_body(c,i,base['slots'],'alternating',book,True)
   else:body,targets=stp_body(c,i,base['slots'],'alternating',shape==1)
   current_cutoffs=cutoffs[:]
   if mode=='cutoff'and 4<=i<8:current_cutoffs=[(1 if v>1 else c['bands'])if v else 0 for v in cutoffs]
   if mode=='mask'and 4<=i<8:current_cutoffs=[v if box%2 else 0 for box,v in enumerate(cutoffs)]
   active=any(current_cutoffs)
   current_cfg=packed(field(3 if base['bands']==64 else 6,4)+'00'+''.join(('1'+field(v,5))if v else'0'for v in current_cutoffs))
   current_ext=packed('0000'+length(len(current_cfg))+raw(current_cfg))if active else b''
   current_ssc=shape_ssc+current_ext;current_control=ordinary+current_ext
   if c['tree']!=7:current_control=packed(raw(current_control)[:18]+'0'+raw(current_control)[19:])
   decoded,differences=payload(c,i,base['slots'],current_cutoffs, 'fill' if mode=='fill' else (bad if i==3 else None))
   if active:body+=bytes([len(decoded)])+decoded if len(decoded)<255 else bytes([255])+int(len(decoded)-255).to_bytes(2,'big')+decoded
   if bad=='spatial-trailing'and i==3:body+=b'\xff'
   changed=i==0 or mode=='repeat' or current_ssc!=previous
   payloads.append(bytes([len(current_ssc)])+current_ssc+body if changed else body);previous=current_ssc
   reference.append(dict(spatial=body.hex(),ssc=current_ssc.hex(),control_ssc=current_control.hex(),cutoffs=current_cutoffs,header=changed,control=(control_body(c,differences if bad!='icc'or i!=3 else [None]*len(differences))if active else control_body(c,differences)[:-1]).hex(),uncorrected_control=control_body(c, [None]*len(differences)).hex(),targets=targets))
  suffixes=[envelope(9,1 if reference[i]['header']else 0,3,b,True)for i,b in enumerate(payloads)]
  if mode=='delayed':suffixes[1:4]=['','', ''.join(suffixes[1:4])]
  if mode=='fragmented':
   half=len(payloads[1])//2;suffixes[1]=envelope(9,0,2,payloads[1][:half],True);suffixes[2]=envelope(9,0,1,payloads[1][half:],True)+suffixes[2]
  frames=[]
  for r,suffix in zip(base['frames'],suffixes):
   prefix=raw(pb[r['offset']:r['offset']+r['bytes']])[:r['prefix_bits']];packet=packed(prefix+suffix);frames.append(dict(offset=len(blob),bytes=len(packet),prefix_bits=len(prefix)));blob.extend(packet)
  name=base['name'].removesuffix('-bands23-on')+f"-v{c['variant']}-bands{c['bands']}-{mode}"+('-bad-'+bad if bad else'')
  out=dict(base,name=name,asc=asc.hex(),ssc=ssc.hex(),ordinary_ssc=ordinary.hex(),control_ssc=control.hex(),mode=mode,variant=c['variant'],parameter_bands=c['bands'],cutoffs=cutoffs,frames=frames,reference=reference)
  (OUT/(name+'.mp4')).write_bytes(video_fixture([out],blob,base['channels'],write=False)['data']);return out
 for base in bases:
  for variant in((0,1)if base['tree']in(2,3,4)else(1,)):
   for bands in(4,9,23):
    c=next(c for c in am if c['tree']==base['tree']and c['variant']==variant and c['bands']==bands)
    for mode in('plain','delayed','fragmented','stp','ges')+(('repeat','cutoff','mask')if bands==23 else()):cases.append(build(base,c,mode))
 base=next(b for b in bases if b['name']=='n512-c1-base-x2-crc1-bands23-on');c=next(c for c in am if c['tree']==7 and c['variant']==1 and c['bands']==23)
 cases.insert(0,build(base,c,'fill'))
 for kind,message in[('reserved13','reserved codebook 13 in MPEG-2 AAC residual'),('short','truncated or oversized bit field'),('spatial-trailing','trailing bytes after LD-MPS spatial frame'),('icc','residual ICC correction exceeds quantization grid')]:
  case=build(base,c,'plain',kind);case['error']=message;errors.append(case)
 case=build(base,c,'delayed','spatial-trailing');case['error']='trailing bytes after LD-MPS spatial frame';errors.append(case)
 # Historical component-domain diagnostics; all native LD profiles here refuse.
 for name,error in [('n512-c1-base-x1-crc1-bands23-on','requires the qualified full-complex64-bin'),('plain-tree7-bands23-on','unqualified residual MDCT-to-QMF update')]:
  base=next(b for b in catalog if b['name']==name);case=build(base,c,'plain');case['error']=error;refusals.append(case)
 (OUT/'n512-c1-base-x2-crc1-v1-bands23-plain-bad-residual-trailing.mp4').unlink(missing_ok=True)
 (OUT/'packets.bin').write_bytes(blob);(OUT/'manifest.json').write_text(json.dumps(dict(scope='Nonconforming LD/residual numerical compositions across eight trees and temporal/transport variants. Every native codec path must reject the profile; raw numerical diagnostics are separate.',cases=cases,errors=errors,refusals=refusals),separators=(',',':'))+'\n');print(len(cases),'nonconforming numerical movies,',len(errors),'errors,',len(refusals),'explicit domain refusals')
if __name__=='__main__':main()
