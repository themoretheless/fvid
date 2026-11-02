import json,re,hashlib
from pathlib import Path
q=Path(__file__).parent
results=json.loads((q/'qualification-results.json').read_text());assert len(results)==7 and all(r['exit_code']==0 for r in results.values())
for name,r in results.items():
    rows=re.findall(r'test result: ok\. (\d+) passed; (\d+) failed; (\d+) ignored',(q/(name+'.log')).read_text())
    r['totals']={key:sum(int(row[i])for row in rows)for i,key in enumerate(['passed','failed','ignored'])};r['log']=str(q/(name+'.log'))
assert sum(r['totals']['passed']for r in results.values())==995
assert sum(r['totals']['ignored']for r in results.values())==2
repeats=json.loads((q/'repeat-results.json').read_text());assert repeats['exit_codes']==[0,0,0]
for i in range(1,4):
    rows=re.findall(r'test result: ok\. (\d+) passed; (\d+) failed; (\d+) ignored',(q/f'repeat-{i}.log').read_text());assert [sum(int(r[j])for r in rows)for j in range(3)]==[53,0,0]
frozen=json.loads((q/'qualification-source.json').read_text());assert all(hashlib.sha256(Path(p).read_bytes()).hexdigest()==h for p,h in frozen.items())
fixtures=json.loads((q/'fixture-verification.json').read_text());assert len(fixtures)==26 and all(hashlib.sha256((Path('tests/fixtures/playback-errors/mp4-alac-edit-subtick-seek')/n).read_bytes()).hexdigest()==h for n,h in fixtures.items())
v=dict(status='accepted',accepted_ids=[],follow_up_ids=['FVID-0484'],count_delta=0,accepted_count=511,next_id='FVID-0512',qualification=results,repeat_runs=3,repeat_passed_each=53,source_frozen=frozen,before=dict(exit_code=101,passed=3,failed=2,log=str(q/'before.log')),existing_tests_unchanged=True,old_ignores=2,refusal=dict(file='tests/fixtures/playback-errors/mp4-alac-edit-subtick-seek/active-short-body.mp4',original_passed=True,final_passed=True,expected_error='ALAC frame is short of its samples',targets=['0ns','500001ns']),limitations=['No speedup or physical audio-device claim.','Mixed codec-family timelines retain their prior paths.','Same initial Duration-to-media-tick loss as FVID-0484;zero new IDs.','Fifteen valid variants and the active refusal control share one existing floor-loss cause.'])
Path('docs/improvements/batch-515-validation.json').write_text(json.dumps(v,indent=2)+'\n')
p=Path('docs/improvements/registry.json');r=json.loads(p.read_text());assert r['accepted_count']==511 and r['next_id']=='FVID-0512'
e=next(e for e in r['entries']if e['id']=='FVID-0484');e['edited_alac_follow_up_validation']='batch-515-validation.json';e['edited_alac_follow_up_note']='Zero-count edited ALAC extension. Pure used ALAC descriptions retain rational Duration floors;mutable native alignment reads true coded counts with bounded header I/O while preserving PCM/AAC/recovered paths. Fifteen raw/Rice/reordered/fill/8k/changing-rate movies verify bit-exact full PCM/video,leading silence,repeated windows,precise suffixes,raw reseek and actual-worker anchors. A separate malformed active packet refuses zero/500001ns seeks before/after. Seven gates pass995 executions,two prior ignores;three repeats pass53 tests each. No speedup or hardware claim.';p.write_text(json.dumps(r,indent=2)+'\n')
(q/'progress.json').write_text(json.dumps(dict(status='accepted',count_delta=0,follow_up_id='FVID-0484',accepted_count=511,next_id='FVID-0512',qualification_test_executions=995,repeat_passed_each=53),indent=2)+'\n')
p=Path('docs/improvements/README.md');s=p.read_text();assert 'Accepted: **511 / 1000'in s
if '## Batch 515'not in s:
    s+='\n## Batch 515 — Edited ALAC native seek precision,zero new IDs\n\nFVID-0484 now retains exact Duration floors in pure ALAC edit timelines. Fifteen authored raw/Rice/reordered/fill/changing-rate movies verify bit-exact PCM/video,leading silence,repeated ranges and actual-worker anchors. A malformed active packet still refuses playback after seek. Seven gates pass995 executions,two prior ignores,and three repeated53-test runs pass. Counter511,next FVID-0512. See `batch-515-validation.json`.\n';p.write_text(s)
print('Accepted zero-count FVID-0484 edited ALAC follow-up;511/1000')
