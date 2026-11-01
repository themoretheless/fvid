from pathlib import Path
import subprocess,json,hashlib,statistics
q=Path(__file__).resolve().parent;baseline=json.loads((q/'baseline-final-binary.json').read_text());old=Path(baseline['path']);new=old.with_name('bench_mp4_uniform_plan_scan');assert hashlib.sha256(old.read_bytes()).hexdigest()==baseline['sha256'];assert old!=new
sources=json.loads((q/'batch-source.json').read_text())
for n,h in sources.items():assert hashlib.sha256(Path(n).read_bytes()).hexdigest()==h,n
binary={k:dict(path=str(p),sha256=hashlib.sha256(p.read_bytes()).hexdigest())for k,p in [('baseline',old),('candidate',new)]}
records=[];results={}
for case,filename,mode in [('bounded-large','frames-262144.mp4','bounded'),('default-large','frames-262144.mp4','default'),('bounded-small','frames-64.mp4','bounded')]:
 source=str(Path('tests/fixtures/playback-errors/mp4-uniform-plan-scan')/filename);pairs=[]
 for pair in range(7):
  values={}
  for version in (['baseline','candidate']if pair%2==0 else ['candidate','baseline']):
   result=subprocess.run([binary[version]['path'],source,'15',mode],check=True,capture_output=True,text=True);data=json.loads(result.stdout);assert 'plan'in data and len(data['elapsed_ns'])==15 and all(n>0 for n in data['elapsed_ns']);assert data['controlled_bytes']==(64<<20 if mode=='bounded'else None);values[version]=data;records.append(dict(case=case,pair=pair,version=version,data=data));(q/'paired-raw.json').write_text(json.dumps(dict(binaries=binary,records=records),indent=2)+'\n')
  assert values['baseline']['plan']==values['candidate']['plan'],(case,pair,'changed plan')
  pairs.append(dict(baseline_ns=statistics.median(values['baseline']['elapsed_ns']),candidate_ns=statistics.median(values['candidate']['elapsed_ns'])))
 before=statistics.median(p['baseline_ns']for p in pairs);after=statistics.median(p['candidate_ns']for p in pairs);results[case]=dict(pairs=pairs,baseline_median_ns=before,candidate_median_ns=after,median_reduction_fraction=1-after/before,winning_pairs=sum(p['candidate_ns']<p['baseline_ns']for p in pairs),plans_identical=True)
criteria=json.loads((q/'criteria.json').read_text());large=results['bounded-large'];default=results['default-large'];passed=large['median_reduction_fraction']>=criteria['required_median_reduction_fraction']and large['winning_pairs']>=criteria['minimum_winning_pairs']and -default['median_reduction_fraction']<=criteria['maximum_default_mode_median_regression_fraction']
for n,h in sources.items():assert hashlib.sha256(Path(n).read_bytes()).hexdigest()==h,n
(q/'measurement-summary.json').write_text(json.dumps(dict(criteria_passed=passed,results=results,binaries=binary,no_decode_or_player_speedup_claim=True),indent=2)+'\n');print(json.dumps(dict(criteria_passed=passed,results=results)),flush=True);assert passed,'Inconclusive optimization:revert or investigate without weakening criteria'
