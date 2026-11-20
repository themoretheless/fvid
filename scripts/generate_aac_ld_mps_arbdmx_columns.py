#!/usr/bin/env python3
"""Own per-input ADG movies and independent sparse-M1 Decimal oracle."""
import json,struct
from pathlib import Path
from decimal import Decimal as D,localcontext
from generate_he_aac_packet_fixtures import field,frequency,packed,video_fixture
from generate_aac_ld_mps_stp_fixtures import config
from generate_aac_ld_mps_entropy_fixtures import pcm
from generate_aac_ld_mps_inband_fixtures import raw,header
from generate_aac_ld_mps_transport_fixtures import envelope,length
from generate_aac_eld_fixtures import mkv
ROOT=Path(__file__).resolve().parents[1];FIX=ROOT/'tests/fixtures/playback-errors';OUT=FIX/'aac-ld-mps-arbdmx-columns'
def main():
 OUT.mkdir(exist_ok=True);parent=FIX/'aac-ld-mps-arbitrary-playback';bases=json.loads((parent/'manifest.json').read_text())['cases'];source=(parent/'packets.bin').read_bytes();models=json.loads((FIX/'aac-ld-mps-assemble/manifest.json').read_text())['cases'];cfgs=(FIX/'aac-ld-mps-assemble/configs.bin').read_bytes();facts=json.loads((FIX/'aac-ld-mps-prepare/facts.json').read_text());blob=bytearray();gold=bytearray();cases=[];controls=[]
 def pack_floats(values):
  at=len(gold);gold.extend(struct.pack('<'+'d'*len(values),*map(float,values)));return at
 for base in bases:
  if base['tree']not in [5,6,7]:continue
  tree=base['tree'];inputs=base['input_channels'];outputs=2 if tree==7 else 8;rows=2 if tree==7 else 8;sources=[0,0]if tree==7 else list(range(6))+([0,1]if tree==5 else[4,5]);old=bytes.fromhex(base['ssc']);ab=raw(bytes.fromhex(base['asc']));marker=field(2,4)+length(len(old))+raw(old)+'000000';at=ab.rfind(marker);assert at>=0;head=ab[:at]
  for bands in [4,5,7,9,12,15,23]:
   model=next(v for v in models if v['tree']==tree and v['variant']==1 and v['bands']==bands);data=cfgs[model['config_offset']:model['config_offset']+model['config_bytes']];plain_ssc=packed(frequency(base['rate'])+field(base['slots']-1,5)+raw(config(model,48000,base['slots'],data,0))[9:]);bits=raw(plain_ssc);pos=18 if tree==7 else 19;assert bits[pos]=='0';ssc=packed(bits[:pos]+'1'+bits[pos+1:]);base_bits='011'
   for r in model['frames'][0]['parameters']:
    offset,levels={'CLD':(15,31),'ICC':(0,8),'CPC':(20,51)}[r['kind']];base_bits+='1111'+'10001'+pcm([r['values'][ps][b]+offset for b in range(len(r['values'][0]))for ps in range(2)],levels)
   base_bits+='00'+'1001';base_wire=packed(base_bits);control_name=base['name']+f'-b{bands}-control';control_packets=[];control_rows=[]
   for i,r in enumerate(base['frames']):
    prefix=raw(source[r['offset']:r['offset']+r['bytes']])[:r['prefix_bits']];packet=packed(prefix+envelope(9,1 if i==0 else 0,3,header(plain_ssc,base_wire)if i==0 else base_wire,True));control_rows.append(dict(offset=len(blob),bytes=len(packet)));blob.extend(packet);control_packets.append(packet)
   control=dict(name=control_name,pcm_offset=0,asc=packed(head+field(2,4)+length(len(plain_ssc))+raw(plain_ssc)+'000000').hex(),slots=base['slots'],bands=base['bands'],channels=outputs,rate=base['rate'],samples=base['samples'],container_rate=base['container_rate'],container_frame_samples=base['container_frame_samples'],frames=control_rows,video=control_name+'.mp4',file=control_name+'.mkv');(OUT/control['video']).write_bytes(video_fixture([control],blob,outputs,write=False)['data']);(OUT/control['file']).write_bytes(mkv(bytes.fromhex(control['asc']),control_packets,base['rate'],base['out_n'],outputs));controls.append(control)
   for pattern in ['unity','varying','switch']:
    previous=[D(0)]*(bands*inputs);references=[];frames=[];packets=[];oracle=[];configs=[]
    for i,r in enumerate(base['frames']):
     enabled=pattern!='switch'or i!=1;config_i=ssc if enabled else plain_ssc;configs.append(config_i.hex());gains=[[[0 if pattern=='unity'or not enabled else[-6,-3,-1,0,1,3,6][(i*2+ps+band+ch)%7]for band in range(bands)]for ps in range(2)]for ch in range(inputs)];wire=base_bits+''.join('1111'+'10001'+pcm([gains[ch][ps][b]+15 for b in range(bands)for ps in range(2)],31)for ch in range(inputs));spatial=packed(wire)if enabled else base_wire;references.append(spatial.hex());prefix=raw(source[r['offset']:r['offset']+r['bytes']])[:r['prefix_bits']];flag=i==0 or pattern=='switch';packet=packed(prefix+envelope(9,1 if flag else 0,3,header(config_i,spatial)if flag else spatial,True));frames.append(dict(offset=len(blob),bytes=len(packet),gains=gains,enabled=enabled));blob.extend(packet);packets.append(packet)
     endpoints=[];slots=[];end_slots=[(base['slots']+1)//2-1,base['slots']-1];last=-1
     with localcontext()as ctx:
      ctx.prec=80
      for ps,end in enumerate(end_slots):
       raw_gain=[D(10)**(D(str(facts['dequantCLD'][gains[ch][ps][band]+15]))/20)for band in range(bands)for ch in range(inputs)];delta=D(end-last)/128;target=raw_gain if ps==0 else[delta*x+(1-delta)*old for x,old in zip(raw_gain,previous)];endpoints.append(pack_floats(target))
       for t in range(last+1,end+1):
        alpha=D(t-last)/D(end-last);slots.append(pack_floats([(1-alpha)*old+alpha*x for old,x in zip(previous,target)]))
       previous=target;last=end
     oracle.append(dict(endpoints=endpoints,slots=slots))
    name=base['name']+f'-b{bands}-{pattern}';asc=packed(head+field(2,4)+length(len(ssc))+raw(ssc)+'000000');c=dict(name=name,pcm_offset=0,asc=asc.hex(),ssc=ssc.hex(),configs=configs,base_ssc=plain_ssc.hex(),base_reference=base_wire.hex(),tree=tree,parameter_bands=bands,inputs=inputs,rows=rows,row_sources=sources,oracle=oracle,reference=references,frames=frames,pattern=pattern,control=control['video'],channels=outputs,rate=base['rate'],slots=base['slots'],bands=base['bands'],out_n=base['out_n'],samples=base['samples'],group=1,container_rate=base['container_rate'],container_frame_samples=base['container_frame_samples'],video=name+'.mp4',file=name+'.mkv');(OUT/c['video']).write_bytes(video_fixture([c],blob,outputs,write=False)['data']);(OUT/c['file']).write_bytes(mkv(asc,packets,base['rate'],base['out_n'],outputs));cases.append(c)
 (OUT/'packets.bin').write_bytes(blob);(OUT/'expected.f64le').write_bytes(gold);(OUT/'manifest.json').write_text(json.dumps(dict(cases=cases,controls=controls,scope='Own ADG input-column gains for trees5/6/7, seven grids, six PCM/shared-LD-SBR domains. Sparse M1 gain endpoints/slots use independent80-digit Decimal recurrence; no foreign whole-decoder oracle.'),separators=(',',':'))+'\n');print('ADG columns:',len(cases),'accepted programmes,',len(controls),'unity controls')
if __name__=='__main__':main()
