"""Compare recorded ABBA seek measurements without changing accounting."""
import argparse,json,statistics
from pathlib import Path
if __name__=='__main__':
 p=argparse.ArgumentParser();p.add_argument('--directory',required=True);a=p.parse_args();root=Path(a.directory);protocol=json.loads((root/'benchmark-protocol.json').read_text());gate=protocol['gates'];campaigns={tag:json.loads((root/f'benchmark-{tag}.json').read_text())for tag in protocol['order']}
 names={r['name']for r in campaigns['A1']['rows']};assert len(names)==8
 for data in campaigns.values():
  assert data['debug_assertions'] and data['rounds']==9 and data['warmup_rounds_excluded']==1
  assert {r['name']for r in data['rows']}==names
  assert all(r['verified'] and r['ns']>0 for r in data['rows'])
 rows=[];passed=True
 for name in sorted(names):
  before=[r for tag in ['A1','A2']for r in campaigns[tag]['rows']if r['name']==name];after=[r for tag in ['B1','B2']for r in campaigns[tag]['rows']if r['name']==name];assert len(before)==len(after)==18
  assert {r['pcm_bytes']for r in before}=={r['pcm_bytes']for r in after}
  bn=statistics.median(r['ns']for r in before);an=statistics.median(r['ns']for r in after);bwork=statistics.median(r['decoded_frame_capacity']for r in before);awork=statistics.median(r['decoded_frame_capacity']for r in after)
  if name.endswith('-late'):ok=bn/an>=gate['late_minimum_median_speedup'] and max(r['decoded_frame_capacity']for r in after)<=gate['late_maximum_decoded_frame_capacity'] and bwork/awork>=gate['late_minimum_decoded_work_reduction']
  else:ok=an/bn<=gate['start_maximum_median_ratio'] and {r['decoded_frame_capacity']for r in before}=={r['decoded_frame_capacity']for r in after}
  passed &=ok;rows.append(dict(name=name,before_median_ns=bn,after_median_ns=an,median_speedup=bn/an,after_over_before=an/bn,before_decoded_frames=bwork,after_decoded_frames=awork,passed=ok))
 report=dict(status='passed'if passed else'failed',samples_per_variant_workload=18,total_timed_executions=288,protocol='benchmark-protocol.json',cases=rows);(root/'benchmark-comparison.json').write_text(json.dumps(report,indent=2)+'\n')
 for r in rows:print(r['name'],'speedup',round(r['median_speedup'],2),'start ratio',round(r['after_over_before'],3),'pass',r['passed'])
 raise SystemExit(0 if passed else 1)
