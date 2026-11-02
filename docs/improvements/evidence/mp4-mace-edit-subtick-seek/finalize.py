import json,re,hashlib
from pathlib import Path
q=Path(__file__).parent
results=json.loads((q/'qualification-results.json').read_text());assert len(results)==7 and all(r['exit_code']==0 for r in results.values())
def totals(p):
 rows=re.findall(r'test result: ok\. (\d+) passed; (\d+) failed; (\d+) ignored',p.read_text())
 return {k:sum(int(row[i])for row in rows)for i,k in enumerate(['passed','failed','ignored'])}
for n,r in results.items():r.update(totals=totals(q/(n+'.log')),log=str(q/(n+'.log')))
assert sum(r['totals']['passed']for r in results.values())==1024
assert sum(r['totals']['failed']for r in results.values())==0
assert sum(r['totals']['ignored']for r in results.values())==2
repeats=json.loads((q/'repeat-results.json').read_text());assert repeats['exit_codes']==[0,0,0]
for i in range(1,4):assert totals(q/f'repeat-{i}.log')==dict(passed=82,failed=0,ignored=0)
assert totals(q/'after-focused.log')==dict(passed=4,failed=0,ignored=0)
frozen=json.loads((q/'qualification-source.json').read_text())
for p,h in frozen.items():assert hashlib.sha256(Path(p).read_bytes()).hexdigest()==h,p
fixtures=json.loads((q/'fixture-verification.json').read_text());assert len(fixtures)==9
for n,h in fixtures.items():assert hashlib.sha256((Path('tests/fixtures/playback-errors/mp4-mace-edit-subtick-seek')/n).read_bytes()).hexdigest()==h,n
v=dict(status='accepted',accepted_ids=[],follow_up_ids=['FVID-0484'],count_delta=0,accepted_count=511,next_id='FVID-0512',qualification=results,repeat_runs=3,repeat_passed_each=82,source_frozen=frozen,before=dict(exit_code=101,passed=2,failed=2,log=str(q/'before.log')),existing_tests_unchanged=True,old_ignores=2,limitations=['No performance or physical audio-device claim.','Mixed PCM/MACE timelines retain their prior routing.','Full natural playback is the seek-history oracle;this is not an independent absolute codec qualification.','Same initial Duration floor loss as FVID-0484;zero new IDs.'])
Path('docs/improvements/batch-518-validation.json').write_text(json.dumps(v,indent=2)+'\n')
p=Path('docs/improvements/registry.json');r=json.loads(p.read_text());assert r['accepted_count']==511 and r['next_id']=='FVID-0512'
e=next(e for e in r['entries']if e['id']=='FVID-0484');e['edited_mace_follow_up_validation']='batch-518-validation.json';e['edited_mace_follow_up_note']='Zero-count edited MACE3/MACE6 extension. Constant sample-clock timelines ceil requested Duration before restoring ordinary decoder history;variable pure MACE timelines preserve rational edit alignment. Eight authored mono/stereo,grouped,changing-description,fragmented/hybrid movies verify silence,repeated ranges,native PCM suffixes/timestamps,raw reseek and actual-worker flush anchors against independently sliced full natural playback. Baseline full PCM/video and raw reseek pass while direct/worker precise seeks fail;unchanged tests pass after. Seven gates include all old MACE suites and pass1024 test executions,two prior ignores;three repeats pass82 tests each. No speedup or physical-device claim.';p.write_text(json.dumps(r,indent=2)+'\n')
(q/'progress.json').write_text(json.dumps(dict(status='accepted',count_delta=0,follow_up_id='FVID-0484',accepted_count=511,next_id='FVID-0512',qualification_test_executions=1024,repeat_passed_each=82),indent=2)+'\n')
p=Path('docs/improvements/README.md');s=p.read_text();assert 'Accepted: **511 / 1000'in s
if '## Batch 518'not in s:
 s+='\n## Batch 518 — Edited MACE native seek precision,zero new IDs\n\nFVID-0484 now preserves exact Duration floors in pure MACE edit timelines,including initial/intermediate silence and repeated source ranges. Eight authored movies compare every native timestamp and PCM byte with independently sliced full natural playback;raw reseek and actual-worker anchors pass. Seven gates include all old MACE suites and pass1024 test executions,two prior ignores;three repeated82-test runs pass. Counter511,next FVID-0512. See `batch-518-validation.json`.\n';p.write_text(s)
p=q/'README.md';s=p.read_text().replace('Pending audit,zero new accepted IDs','Accepted zero-count FVID-0484 follow-up').replace('Seven broader gates,including all three old MACE suites,and three repeats are pending.','All seven broader gates,including all three old MACE suites,pass1024 test executions,two prior ignores;three repeated82-test runs pass on frozen source.');p.write_text(s)
print('Accepted zero-count FVID-0484 edited MACE follow-up;511/1000')
