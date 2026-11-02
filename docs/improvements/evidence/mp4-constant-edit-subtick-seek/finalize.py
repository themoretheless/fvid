import json,re,hashlib
from pathlib import Path
q=Path(__file__).parent
results=json.loads((q/'qualification-results.json').read_text());assert len(results)==7 and all(r['exit_code']==0 for r in results.values())
def totals(p):
 rows=re.findall(r'test result: ok\. (\d+) passed; (\d+) failed; (\d+) ignored',p.read_text())
 return {k:sum(int(row[i])for row in rows)for i,k in enumerate(['passed','failed','ignored'])}
for n,r in results.items():r.update(totals=totals(q/(n+'.log')),log=str(q/(n+'.log')))
assert sum(r['totals']['passed']for r in results.values())==1047
assert sum(r['totals']['failed']for r in results.values())==0
assert sum(r['totals']['ignored']for r in results.values())==2
repeats=json.loads((q/'repeat-results.json').read_text());assert repeats['exit_codes']==[0,0,0]
for i in range(1,4):assert totals(q/f'repeat-{i}.log')==dict(passed=105,failed=0,ignored=0)
assert totals(q/'after-raw-anchor.log')==dict(passed=4,failed=0,ignored=0)
frozen=json.loads((q/'qualification-source.json').read_text())
for p,h in frozen.items():assert hashlib.sha256(Path(p).read_bytes()).hexdigest()==h,p
fixtures=json.loads((q/'fixture-verification.json').read_text());assert len(fixtures)==9
for n,h in fixtures.items():assert hashlib.sha256((Path('tests/fixtures/playback-errors/mp4-constant-edit-subtick-seek')/n).read_bytes()).hexdigest()==h,n
assert 'test result: FAILED. 2 passed; 2 failed; 0 ignored;' in (q/'before.log').read_text()
assert 'test result: FAILED. 3 passed; 1 failed; 0 ignored;' in (q/'after-focused.log').read_text()
assert json.loads((q/'test-integrity.json').read_text())['new_tests_and_generator_identical_before_after']
stamps=json.loads((q/'stamp-results.json').read_text());assert stamps['status']=='qualified' and stamps['before_exit_code']==101 and stamps['after_exit_code']==0 and stamps['repeat_exit_codes']==[0,0,0] and stamps['candidate_restored'] and stamps['backup_removed']
assert 'test result: FAILED. 0 passed; 1 failed;' in (q/'stamps-before.log').read_text()
for n in ['stamps-after.log','stamps-repeat-1.log','stamps-repeat-2.log','stamps-repeat-3.log']:assert totals(q/n)==dict(passed=1,failed=0,ignored=0)
for p,h in json.loads((q/'stamp-source.json').read_text()).items():
 assert hashlib.sha256(Path(p).read_bytes()).hexdigest()==h,p
 if p in frozen:assert frozen[p]==h
 frozen[p]=h
baseline=json.loads((q/'baseline-case-classification.json').read_text());assert baseline['precise_seek_failures']==['ima4','ima-wav','ms-adpcm','ac3','ac3-grouped','ac3-one-packet'];assert baseline['passing_seek_controls']==['eac3-48000','eac3-32000']
v=dict(status='accepted',accepted_ids=[],follow_up_ids=['FVID-0484'],count_delta=0,accepted_count=511,next_id='FVID-0512',qualification=results,strict_stamp_grid=stamps,strict_stamp_passed=1,strict_stamp_repeat_runs=3,qualification_test_executions=1048,repeat_runs=3,repeat_passed_each=105,source_frozen=frozen,before=dict(exit_code=101,passed=2,failed=2,log=str(q/'before.log')),baseline_cases=baseline,first_candidate=dict(exit_code=101,passed=3,failed=1,log=str(q/'after-focused.log'),cause='Raw clock stamp collides with a stale precise anchor on the coarse media clock;fixed without changing the test.'),existing_tests_unchanged=True,old_ignores=2,limitations=['No performance or physical audio-device claim.','Natural full playback is the seek-history oracle;this is not an independent absolute decoder qualification.','Both EAC3 cases already passed before the fix and are controls,not additional defects.','Same Duration-floor cause as FVID-0484;zero new IDs.'])
Path('docs/improvements/batch-519-validation.json').write_text(json.dumps(v,indent=2)+'\n')
p=Path('docs/improvements/registry.json');r=json.loads(p.read_text());assert r['accepted_count']==511 and r['next_id']=='FVID-0512'
e=next(e for e in r['entries']if e['id']=='FVID-0484');e['constant_edit_follow_up_validation']='batch-519-validation.json';e['constant_edit_follow_up_note']='Zero-count constant AudioTimeline extension. Native sample cursors now ceil exact Duration for ADPCM and AC3 edit lists,retaining normal decoder history. Raw timeline navigation clears a stale precise anchor;the first candidate failing the unchanged raw guard is preserved. Eight authored movies verify full PCM/video,opening/intermediate silence,repeated source ranges,native timestamps/PCM suffixes,raw reseek and actual-worker anchors. Baseline failures cover three ADPCM formats and three AC3 groupings;two EAC3 rates already pass as controls. Seven gates include19 older ADPCM/Dolby subsystem tests and pass1047 test executions,two old ignores;three repeats pass105 tests each. An independent direct/actual-worker packet-stamp gate compares raw Duration timestamps against integer native cursors without normalization;it fails on the original source,passes after and repeats three times. No new ID,speedup or physical-device claim.';p.write_text(json.dumps(r,indent=2)+'\n')
(q/'progress.json').write_text(json.dumps(dict(status='accepted',count_delta=0,follow_up_id='FVID-0484',accepted_count=511,next_id='FVID-0512',qualification_test_executions=1048,main_qualification_test_executions=1047,strict_stamp_passed=1,strict_stamp_repeat_runs=3,repeat_passed_each=105),indent=2)+'\n')
p=Path('docs/improvements/README.md');s=p.read_text();assert 'Accepted: **511 / 1000'in s
if '## Batch 519'not in s:
 s+='\n## Batch 519 — Constant ADPCM/AC3 edit seek precision,zero new IDs\n\nFVID-0484 now preserves exact Duration floors for constant AudioTimeline cursors. Six authored ADPCM/AC3 cases reproduce the failure;two EAC3 rates are passing controls. All eight verify natural PCM/video,silence,repeats,exact native suffixes,raw clock restoration and actual-worker anchors. Seven gates include19 older ADPCM/Dolby tests and pass1047 test executions,two prior ignores;three repeated105-test runs pass. One independent strict packet-stamp test fails before,passes after and repeats three times,including48k native cursors. Counter511,next FVID-0512. See `batch-519-validation.json`.\n';p.write_text(s)
p=q/'README.md';s=p.read_text().replace('Pending audit,zero accepted IDs','Accepted zero-count FVID-0484 follow-up').replace('Seven broader gates now include19 existing ADPCM/Dolby subsystem tests;three repetitions are also required.','All seven broader gates include19 existing ADPCM/Dolby subsystem tests and pass1047 test executions,two prior ignores;three repeated105-test runs pass on frozen source. An additional independent strict packet-stamp gate fails before,passes after and repeats three times.');p.write_text(s)
print('Accepted zero-count FVID-0484 constant edit follow-up;511/1000')
