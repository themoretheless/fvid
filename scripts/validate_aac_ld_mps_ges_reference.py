#!/usr/bin/env python3
"""Check reference activity; explicit audit, not fixture/test generation."""
import json,struct,hashlib
from pathlib import Path
ROOT=Path(__file__).resolve().parents[1];OUT=ROOT/'tests/fixtures/playback-errors/aac-ld-mps-ges'
def main():
 m=json.loads((OUT/'component.json').read_text());o=json.loads((OUT/'component-oracle.json').read_text());raw=(OUT/'component-input.f32le').read_bytes();expected=(OUT/'component-expected.f32le').read_bytes();gap=0.;changed=low=high=0
 for c,oracle in zip(m['cases'],o['cases']):
  for b,ref in zip(c['blocks'],oracle['blocks']):
   n=b['slots']*c['outputs']*c['bands']*2;at=b['offset']+b['slots']*c['inputs']*c['bands']*8;end=ref['expected_offset'];original=raw[at:at+n*4];result=expected[end:end+n*4]
   if b['targets']is None:assert original==result,'disabled reference changed dry data'
   for row in range(b['slots']*c['outputs']):assert original[row*c['bands']*8:row*c['bands']*8+48]==result[row*c['bands']*8:row*c['bands']*8+48],'low-bin reference changed'
   for (a,),(z,)in zip(struct.iter_unpack('<f',original),struct.iter_unpack('<f',result)):
    delta=abs(a-z);gap=max(gap,delta);changed+=delta>1e-9
    if abs(a)>1e-9:
     factor=z/a;low+=abs(factor-.25)<1e-6;high+=abs(factor-4)<1e-6
 assert gap>1e-4 and changed>1000 and low>1000 and high>1000
 report=dict(scope='Reference quality/activity audit: identity shaping must fail the component comparison; disabled/low-bin data remains exact.',identity_negative_control_gap=gap,modified_scalar_values=changed,lower_clamp_scalar_values=low,upper_clamp_scalar_values=high,expected_sha256=hashlib.sha256(expected).hexdigest(),audit_sha256=hashlib.sha256(Path(__file__).read_bytes()).hexdigest())
 (ROOT/'docs/validation/aac-ld-mps-ges-reference-activity-2026-10-30.json').write_text(json.dumps(report,indent=2)+'\n');print(report)
if __name__=='__main__':main()
