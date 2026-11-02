import json,re,hashlib
from pathlib import Path
q=Path(__file__).parent
results=json.loads((q/'qualification-results.json').read_text());assert len(results)==7 and all(r['exit_code']==0 for r in results.values())
def totals(p):
 rows=re.findall(r'test result: ok\. (\d+) passed; (\d+) failed; (\d+) ignored',p.read_text())
 return {k:sum(int(row[i])for row in rows)for i,k in enumerate(['passed','failed','ignored'])}
for n,r in results.items():r.update(totals=totals(q/(n+'.log')),log=str(q/(n+'.log')))
assert sum(r['totals']['passed']for r in results.values())==1004
assert sum(r['totals']['failed']for r in results.values())==0
assert sum(r['totals']['ignored']for r in results.values())==2
related=json.loads((q/'related-results.json').read_text());assert related['exit_code']==0;assert totals(q/'related.log')==dict(passed=16,failed=0,ignored=0)
repeats=json.loads((q/'repeat-results.json').read_text());assert repeats['exit_codes']==[0,0,0]
for i in range(1,4):assert totals(q/f'repeat-{i}.log')==dict(passed=78,failed=0,ignored=0)
assert totals(q/'after-focused.log')==dict(passed=4,failed=0,ignored=0)
frozen=json.loads((q/'qualification-source.json').read_text());frozen.update(json.loads((q/'related-source.json').read_text()))
for p,h in frozen.items():assert hashlib.sha256(Path(p).read_bytes()).hexdigest()==h,p
fixtures=json.loads((q/'fixture-verification.json').read_text());assert len(fixtures)==11
for n,h in fixtures.items():assert hashlib.sha256((Path('tests/fixtures/playback-errors/mp4-mace-subtick-seek')/n).read_bytes()).hexdigest()==h,n
v=dict(status='accepted',accepted_ids=[],follow_up_ids=['FVID-0484'],count_delta=0,accepted_count=511,next_id='FVID-0512',qualification=results,related=related,related_passed=16,repeat_runs=3,repeat_passed_each=78,source_frozen=frozen,before=dict(exit_code=101,passed=2,failed=2,log=str(q/'before.log')),existing_tests_unchanged=True,old_ignores=2,limitations=['No performance or physical audio-device claim.','Edited and mixed PCM/MACE timelines retain their prior routing.','Natural full playback is the seek-history oracle;this is not an independent absolute MACE decoder qualification.','Same initial Duration floor loss as FVID-0484;zero new IDs.'])
Path('docs/improvements/batch-517-validation.json').write_text(json.dumps(v,indent=2)+'\n')
p=Path('docs/improvements/registry.json');r=json.loads(p.read_text());assert r['accepted_count']==511 and r['next_id']=='FVID-0512'
e=next(e for e in r['entries']if e['id']=='FVID-0484');e['mace_follow_up_validation']='batch-517-validation.json';e['mace_follow_up_note']='Zero-count unedited MACE3/MACE6 extension. Native landing uses coded block frame counts and retains precise Duration floors/anchors without changing stateful preroll or setup boundaries. Ten authored mono/stereo,grouped,changing-description,fragmented/reordered/hybrid movies compare every native timestamp and PCM byte with independently sliced complete natural playback. Full PCM/video and raw-origin controls pass before/after;precise seek and actual-worker anchors fail before and pass after. Seven gates pass1004 test executions,two old ignores;16 additional MACE subsystem tests pass;three repeats pass78 tests each. Edited/mixed-family routing and absolute codec qualification are outside this proof;no speedup or physical-device claim.';p.write_text(json.dumps(r,indent=2)+'\n')
(q/'progress.json').write_text(json.dumps(dict(status='accepted',count_delta=0,follow_up_id='FVID-0484',accepted_count=511,next_id='FVID-0512',qualification_test_executions=1004,repeat_passed_each=78),indent=2)+'\n')
p=Path('docs/improvements/README.md');s=p.read_text();assert 'Accepted: **511 / 1000'in s
if '## Batch 517'not in s:
 s+='\n## Batch 517 — MACE native seek precision,zero new IDs\n\nFVID-0484 now retains precise Duration floors in unedited pure MACE3/MACE6 tracks without losing decoder history. Ten authored mono/stereo,grouped,changing-description and fragmented/reordered/hybrid movies compare exact PCM suffixes and native timestamps with full natural playback;actual-worker anchors and raw reseeks pass. Seven gates pass1004 test executions,two old ignores;16 additional MACE subsystem tests pass,and three repeated78-test runs pass. Counter511,next FVID-0512. See `batch-517-validation.json`.\n';p.write_text(s)
p=q/'README.md';s=p.read_text().replace('Pending zero-count','Accepted zero-count').replace('Seven qualification gates and three repeats are pending.','All seven qualification gates pass1004 test executions,two prior ignores;16 additional MACE subsystem tests pass;three repeated78-test runs pass on frozen source.');p.write_text(s)
print('Accepted zero-count FVID-0484 MACE follow-up;511/1000')
