#!/usr/bin/env python3
"""Own in-band spatial-header movies. Explicit offline generation, no codecs."""
import json,struct
from pathlib import Path
from generate_aac_ld_mps_transport_fixtures import envelope,packed,field
from generate_he_aac_packet_fixtures import video_fixture
ROOT=Path(__file__).resolve().parents[1]
FIX=ROOT/'tests/fixtures/playback-errors'
OUT=FIX/'aac-ld-mps-inband'
def raw(data):return ''.join(field(v,8)for v in data)
def gain_changed(config,tree):
 bits=raw(config);at=19 if tree==7 else 26
 return packed(bits[:at]+field((int(bits[at:at+3],2)+1)%8,3)+bits[at+3:])
def header(config,body,extended=False):
 if extended:config+=bytes([0xf0])*(129-len(config))
 size=len(config)
 return bytes([size])+config+body if size<127 else bytes([127])+struct.pack('>H',size-127)+config+body

def main():
 OUT.mkdir(exist_ok=True)
 source=FIX/'aac-eld-mps-core';bases=[]
 for tree in range(8):
  prefixes=[]
  for i in range(3):
   count,_=struct.unpack('<II',(source/f'tree{tree}-{i}.bits').read_bytes())
   prefixes.append(raw((source/f'tree{tree}-{i}.au').read_bytes())[:count])
  bases.append(dict(name=f'plain-tree{tree}',asc=(source/f'tree{tree}.spatial.asc').read_bytes().hex(),tree=tree,n=512,rate=48000,slots=8,bands=64,channels=1 if tree in(0,1,7)else 2 if tree in(2,3,4)else 6,config=(source/f'tree{tree}.ssc').read_bytes(),body=(source/f'tree{tree}-0.spatial').read_bytes(),prefixes=prefixes,qmf=None))
 coupled=FIX/'aac-eld-spatial-sbr';manifest=json.loads((coupled/'manifest.json').read_text());blob=(coupled/'packets.bin').read_bytes()
 assembly=FIX/'aac-ld-mps-assemble';am=json.loads((assembly/'manifest.json').read_text())['cases'];configs=(assembly/'configs.bin').read_bytes();parameters=(assembly/'packets.bin').read_bytes()
 for case in manifest['cases']:
  if case['mode']!='base' or 'crc1' not in case['name']:continue
  tree=int(case['name'].split('-tree')[1])if '-tree' in case['name']else {1:7,2:2,6:5}[case['configuration']]
  spatial=next(c for c in am if c['tree']==tree and c['variant']==1)
  cfg=configs[spatial['config_offset']:spatial['config_offset']+spatial['config_bytes']]
  cfg=packed(field(3 if case['bands']==64 else 6,4)+field(case['slots']-1,5)+raw(cfg)[9:])
  f=spatial['frames'][0];body=parameters[f['offset']:f['offset']+f['bytes']]
  prefixes=[raw(blob[r['offset']:r['offset']+r['bytes']])[:r['core_bits']+r['sbr_bits']]for r in case['frames'][:3]]
  bases.append(dict(name=case['name'],asc=case['asc'],tree=tree,n=case['n'],rate=case['rate'],slots=case['slots'],bands=case['bands'],channels=case['channels'],config=cfg,body=body,prefixes=prefixes,qmf={k:case[k]for k in('expected_offset','expected_bands')}))
 blob=bytearray();cases=[];errors=[]
 for base in bases:
  initial=base['config'];changed=gain_changed(initial,base['tree']);body=base['body']
  for mode in ('same','repeat','extended','aligned-zero','gain','late-gain','fragmented-gain'):
   phase=[0,1,0]if 'gain' in mode else [0,0,0]
   payloads=[header([initial,changed][p],body,mode=='extended')for p in phase]
   suffixes=[envelope(9,1,3,p,True)for p in payloads]
   if mode=='aligned-zero':suffixes=[envelope(9,1,3,bytes([128+len(initial)])+initial+b'\0\0'+body,True)]*3
   if mode=='same':suffixes[1:]=[envelope(9,0,3,body,True)]*2
   if mode=='late-gain':suffixes=['','', ''.join(suffixes)]
   if mode=='fragmented-gain':
    half=len(payloads[0])//2
    suffixes=[envelope(9,1,2,payloads[0][:half],True),envelope(9,1,1,payloads[0][half:],True)+suffixes[1],suffixes[2]]
   frames=[]
   for prefix,suffix in zip(base['prefixes'],suffixes):
    packet=packed(prefix+suffix);frames.append(dict(offset=len(blob),bytes=len(packet),prefix_bits=len(prefix)));blob.extend(packet)
   case={k:base[k]for k in('asc','tree','n','rate','slots','bands','channels','qmf')}
   case.update(name=base['name']+'-'+mode,mode=mode,configs=[initial.hex(),changed.hex()],body=body.hex(),phase=phase,frames=frames,container_rate=base['rate'],container_frame_samples=base['slots']*base['bands'],samples=3*base['slots']*base['bands'],pcm_offset=0)
   (OUT/(case['name']+'.mp4')).write_bytes(video_fixture([case],blob,base['channels'],write=False)['data']);cases.append(case)
  if base['name'] not in ('plain-tree7','n480-c1-base-x2-crc1'):continue
  wrong=packed(field(6 if base['rate']==48000 else 3,4)+raw(initial)[4:])
  grid=packed(raw(initial)[:9]+field(2 if raw(initial)[9:12]!='010' else 1,3)+raw(initial)[12:])
  malformed=[('grid-change',header(grid,body),'matrix grid change'),('empty',bytes([0]),'empty in-band'),('short',bytes([126])+initial,'truncated LD-MPS configuration'),('extended-short',bytes([127,0]),'truncated extended in-band'),('missing-frame',header(initial,b''),'missing spatial frame'),('time-alignment',bytes([128+len(initial)])+initial+b'\0\1'+body,'time alignment'),('alignment-short',bytes([128+len(initial)])+initial+b'\0','truncated in-band LD-MPS time alignment'),('wrong-rate',header(wrong,body),'output format change'),('reserved-config',header(bytes([0xd0])+bytes(len(initial)-1),body),'reserved')]
  for mode,payload,error in malformed:
   frames=[]
   for i,prefix in enumerate(base['prefixes']):
    suffix=envelope(9,1,3,payload if i==1 else header(initial,body),True)
    packet=packed(prefix+suffix);frames.append(dict(offset=len(blob),bytes=len(packet),prefix_bits=len(prefix)));blob.extend(packet)
   case=dict(cases[-1],name=base['name']+'-bad-'+mode,mode=mode,phase=[0,0,0],frames=frames,error=error)
   (OUT/(case['name']+'.mp4')).write_bytes(video_fixture([case],blob,base['channels'],write=False)['data']);errors.append(case)
 (OUT/'packets.bin').write_bytes(blob)
 (OUT/'manifest.json').write_text(json.dumps(dict(scope='Own HeaderAndFrame prefix, identical-header continuity, gain change/restore, delayed and fragmented transport, reset/seek and bounded errors. No private media or foreign codec.',cases=cases,errors=errors),separators=(',',':'))+'\n')
 print(len(bases),'base configurations,',len(cases),'valid,',len(errors),'malformed')
if __name__=='__main__':main()
