#!/usr/bin/env python3
"""Own SBR/PS ASC output clocks without Matroska OutputSamplingFrequency."""
import json, hashlib, struct
from generate_he_aac_packet_fixtures import DEST, asc
from generate_aac_ps_output_clock_matroska_fixtures import element

def fields(data):
 pos=0
 while pos<len(data):
  start=pos; width=next(w for w in range(1,5) if data[pos] & (1 << (8-w)))
  tag=int.from_bytes(data[pos:pos+width],'big');pos+=width
  width=next(w for w in range(1,9) if data[pos] & (1 << (8-w)))
  size=int.from_bytes(data[pos:pos+width],'big') ^ (1 << (7*width));pos+=width
  yield tag,data[pos:pos+size]
  pos+=size
 assert pos==len(data)

def rewrite(data,private,keep_output=False):
 out=b''
 for tag,body in fields(data):
  if tag==0x78b5 and not keep_output:continue
  if tag in [0x18538067,0x1654ae6b,0xae,0xe1]:body=rewrite(body,private,keep_output)
  if tag==0x63a2 and body[:1]!=b'\x01':body=private
  out+=element(tag,body)
 return out

def main():
 source=json.loads((DEST/'aac-ps-output-clock-matroska-oracles.json').read_text());cases=[]
 for row in source['cases']:
  if '-sbr-' not in row['file']:continue
  for signalling in ['explicit','sync']:
   for ps in [False,True]:
    private=asc(24000,48000,row['slots'],signalling,ps=ps)
    data=rewrite((DEST/row['file']).read_bytes(),private)
    name=f"he-aac-ps-asc-clock-{'ps' if ps else 'sbr'}-{signalling}-{row['slots']*64}-synthetic.mkv"
    (DEST/name).write_bytes(data)
    cases.append(dict(row,file=name,sha256=hashlib.sha256(data).hexdigest(),asc=private.hex()))
 controls=[]
 seed=next(c for c in source['cases'] if '-sbr-' in c['file']);original=(DEST/seed['file']).read_bytes()
 implicit=json.loads((DEST/'aac-ps-inband-oracles.json').read_text())
 lc=next(c for c in implicit['cases'] if c['kind']=='LC' and c['slots']==seed['slots'])
 variants=[('unspecified',rewrite(original,bytes.fromhex(lc['video']['asc'])),24000),
           ('explicit-core',original.replace(element(0x78b5,struct.pack('>d',48000)),element(0x78b5,struct.pack('>d',24000))),24000),
           ('conflicting-core',rewrite(original,asc(24000,48000,seed['slots'],'explicit')).replace(element(0xb5,struct.pack('>d',24000)),element(0xb5,struct.pack('>d',32000))),32000)]
 for label,data,rate in variants:
  name=f'he-aac-ps-asc-clock-control-{label}-synthetic.mkv';(DEST/name).write_bytes(data)
  controls.append(dict(file=name,sample_rate=rate,sha256=hashlib.sha256(data).hexdigest()))
 (DEST/'aac-ps-asc-clock-matroska-oracles.json').write_text(json.dumps(dict(cases=cases,controls=controls),indent=2)+'\n')
 print('ASC clock fixtures:',len(cases))
if __name__=='__main__':main()
