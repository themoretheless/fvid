#!/usr/bin/env python3
"""Interleave preserved release executables; do not impose timing gates on tests."""
import sys,json,subprocess,hashlib,statistics
from pathlib import Path
def main():
 baseline,after,workload=map(Path,sys.argv[1:4]);queries=int(sys.argv[4]) if len(sys.argv)>4 else 1000;rounds=int(sys.argv[5]) if len(sys.argv)>5 else 7
 assert queries>0 and rounds>=7
 bins={'baseline':baseline,'after':after}
 def run(k):
  p=subprocess.run([str(bins[k]),str(workload),str(queries)],capture_output=True,text=True);assert p.returncode==0,(k,p.stderr)
  r=json.loads(p.stdout);assert r['queries']==queries and r['packets']>=3;return r
 warmup={k:run(k) for k in bins};samples=[]
 for i in range(rounds):
  order=['baseline','after'] if i%2==0 else ['after','baseline'];row={'round':i,'order':order}
  for k in order:row[k]=run(k)
  assert row['baseline']['packets']==row['after']['packets'];samples.append(row)
 stats={}
 for k in bins:
  stats[k]={}
  for metric in ['open_ns','navigation_ns','seek_rgb_ns']:
   a=[r[k][metric] for r in samples];m=statistics.median(a);stats[k][metric]={'median':m,'mad':statistics.median(abs(x-m) for x in a),'minimum':min(a),'maximum':max(a)}
 ratios={m:stats['baseline'][m]['median']/stats['after'][m]['median'] for m in ['open_ns','navigation_ns','seek_rgb_ns']}
 wins={m:sum(r['after'][m]<r['baseline'][m] for r in samples) for m in ratios}
 accepted=ratios['navigation_ns']>=1.2 and ratios['seek_rgb_ns']>=1.1 and wins['navigation_ns']>=rounds-1 and wins['seek_rgb_ns']>=rounds-1 and ratios['open_ns']>=1/1.1
 print(json.dumps({'scope':'Memory-resident input; public software sync navigation and actual seek+RGB with exact frame/interval assertions. No disk/GPU/whole-player claim.','criteria':'Navigation median>=1.2x,seek+RGB median>=1.1x,both win all but at most1 paired rounds,open median regresses<=10%; criteria fixed before implementation.','accepted_by_criteria':accepted,'queries':queries,'rounds':rounds,'binary_sha256':{k:hashlib.sha256(p.read_bytes()).hexdigest() for k,p in bins.items()},'workload_sha256':hashlib.sha256(workload.read_bytes()).hexdigest(),'warmup_excluded':warmup,'samples':samples,'statistics':stats,'baseline_over_after':ratios,'paired_wins':wins},indent=2))
if __name__=='__main__':main()
