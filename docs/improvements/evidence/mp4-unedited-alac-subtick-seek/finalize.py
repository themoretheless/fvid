import json,re,hashlib
from pathlib import Path
q=Path(__file__).parent
results=json.loads((q/'qualification-results.json').read_text());assert len(results)==7 and all(r['exit_code']==0 for r in results.values())
for name,r in results.items():
    rows=re.findall(r'test result: ok\. (\d+) passed; (\d+) failed; (\d+) ignored',(q/(name+'.log')).read_text())
    r['totals']={key:sum(int(row[i])for row in rows)for i,key in enumerate(['passed','failed','ignored'])};r['log']=str(q/(name+'.log'))
assert sum(r['totals']['passed']for r in results.values())==990
assert sum(r['totals']['ignored']for r in results.values())==2
repeats=json.loads((q/'repeat-results.json').read_text());assert repeats['exit_codes']==[0,0,0]
for i in range(1,4):
    rows=re.findall(r'test result: ok\. (\d+) passed; (\d+) failed; (\d+) ignored',(q/f'repeat-{i}.log').read_text());assert [sum(int(r[j])for r in rows)for j in range(3)]==[48,0,0]
frozen=json.loads((q/'qualification-source.json').read_text());assert all(hashlib.sha256(Path(p).read_bytes()).hexdigest()==h for p,h in frozen.items())
fixtures=json.loads((q/'fixture-verification.json').read_text());assert len(fixtures)==18 and all(hashlib.sha256((Path('tests/fixtures/playback-errors/mp4-unedited-alac-subtick-seek')/n).read_bytes()).hexdigest()==h for n,h in fixtures.items())
v=dict(status='accepted',accepted_ids=[],follow_up_ids=['FVID-0484'],count_delta=0,accepted_count=511,next_id='FVID-0512',qualification=results,repeat_runs=3,repeat_passed_each=48,source_frozen=frozen,before=dict(exit_code=101,passed=3,failed=2,log=str(q/'before.log')),existing_tests_unchanged=True,old_ignores=2,refusal=dict(file='tests/fixtures/playback-errors/mp4-unedited-alac-subtick-seek/header-valid-body-short.mp4',original_passed=True,final_passed=True,expected_error='ALAC frame is short of its samples',targets=['60ms','DurationMAX']),limitations=['No speedup or physical audio-device claim.','Edited ALAC and mixed codec-family timelines retain their prior paths.','Same initial Duration-to-media-tick loss as FVID-0484;zero new IDs.','Generator chunk-table and expected-error staging repairs add zero IDs.'])
Path('docs/improvements/batch-514-validation.json').write_text(json.dumps(v,indent=2)+'\n')
p=Path('docs/improvements/registry.json');r=json.loads(p.read_text());assert r['accepted_count']==511 and r['next_id']=='FVID-0512'
e=next(e for e in r['entries']if e['id']=='FVID-0484');e['unedited_alac_follow_up_validation']='batch-514-validation.json';e['unedited_alac_follow_up_note']='Zero-count unedited ALAC extension. Retain rational Duration floors and derive selected native landings from real ALAC first-element sample counts. Bounded eight-byte range probing falls back under normal packet admission for ancillary elements;full decode/body validation remains. Nine authored raw/Rice/media-clock/reordered/fill movies preserve independent full PCM/video,precise suffixes,raw reseek and actual-worker anchors. A separate short-body refusal passes before/after at EOF and DurationMAX. Seven gates pass990 executions,two prior ignores;three repeats pass48 tests each. No speedup or hardware claim.';p.write_text(json.dumps(r,indent=2)+'\n')
(q/'progress.json').write_text(json.dumps(dict(status='accepted',count_delta=0,follow_up_id='FVID-0484',accepted_count=511,next_id='FVID-0512',qualification_test_executions=990,repeat_passed_each=48),indent=2)+'\n')
p=Path('docs/improvements/README.md');s=p.read_text();assert 'Accepted: **511 / 1000'in s
if '## Batch 514'not in s:
    s+='\n## Batch 514 — Unedited ALAC seek precision,zero new IDs\n\nFVID-0484 now retains exact Duration floors for unedited ALAC and computes its native anchor from the actual coded sample-count header. Nine valid authored raw/Rice/media-clock/reordered/fill movies preserve full PCM/video and raw navigation;the separate truncated-body movie still refuses EOF seeks after its valid header. Seven gates pass990 executions,two prior ignores,and three repeated48-test runs pass. No performance claim. Counter511,next FVID-0512. See `batch-514-validation.json`.\n';p.write_text(s)
print('Accepted zero-count FVID-0484 ALAC follow-up;511/1000')
