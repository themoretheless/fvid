import json,re,hashlib
from pathlib import Path
q=Path(__file__).parent
results=json.loads((q/'qualification-results.json').read_text());assert len(results)==7 and all(r['exit_code']==0 for r in results.values())
def totals(p):
 rows=re.findall(r'test result: ok\. (\d+) passed; (\d+) failed; (\d+) ignored',p.read_text())
 return {k:sum(int(row[i])for row in rows)for i,k in enumerate(['passed','failed','ignored'])}
for n,r in results.items():r.update(totals=totals(q/(n+'.log')),log=str(q/(n+'.log')))
assert sum(r['totals']['passed']for r in results.values())==1061
assert sum(r['totals']['failed']for r in results.values())==0
assert sum(r['totals']['ignored']for r in results.values())==2
repeats=json.loads((q/'repeat-results.json').read_text());assert repeats['exit_codes']==[0,0,0]
for i in range(1,4):assert totals(q/f'repeat-{i}.log')==dict(passed=119,failed=0,ignored=0)
assert totals(q/'after-focused.log')==dict(passed=4,failed=0,ignored=0)
frozen=json.loads((q/'qualification-source.json').read_text())
for p,h in frozen.items():assert hashlib.sha256(Path(p).read_bytes()).hexdigest()==h,p
fixtures=json.loads((q/'fixture-verification.json').read_text());assert len(fixtures)==17
for n,h in fixtures.items():assert hashlib.sha256((Path('tests/fixtures/playback-errors/mp4-mixed-native-seek')/n).read_bytes()).hexdigest()==h,n
assert 'test result: FAILED. 2 passed; 2 failed; 0 ignored;' in (q/'before.log').read_text()
baseline=json.loads((q/'baseline-cases.json').read_text());assert len(baseline['precise_seek_failures'])==16 and baseline['full_pcm_video_and_raw_reseek_all_passed']
assert json.loads((q/'test-integrity.json').read_text())['new_tests_and_generator_identical_before_after']
v=dict(status='accepted',accepted_ids=[],follow_up_ids=['FVID-0484'],count_delta=0,accepted_count=511,next_id='FVID-0512',qualification=results,repeat_runs=3,repeat_passed_each=119,source_frozen=frozen,before=dict(exit_code=101,passed=2,failed=2,log=str(q/'before.log')),baseline_cases=baseline,existing_tests_unchanged=True,old_ignores=2,limitations=['No performance or physical audio-device claim.','The natural-playback oracle qualifies seek history,not an independent absolute codec decoder.','All new witness rates are8k/16k with exact integer-nanosecond periods;actual packet timestamps are compared without native-index normalization.','Mixed AAC/PCM and ALAC/PCM timelines retain their prior routing and are not certified by this follow-up.','Same Duration-floor cause as FVID-0484;zero new IDs.'])
Path('docs/improvements/batch-520-validation.json').write_text(json.dumps(v,indent=2)+'\n')
p=Path('docs/improvements/registry.json');r=json.loads(p.read_text());assert r['accepted_count']==511 and r['next_id']=='FVID-0512'
e=next(e for e in r['entries']if e['id']=='FVID-0484');e['mixed_native_follow_up_validation']='batch-520-validation.json';e['mixed_native_follow_up_note']='Zero-count mixed PCM/MACE/ADPCM extension. All used descriptions supported by the existing per-packet native frame counter now share precise Duration routing;unused metadata stays irrelevant and decoder history/setup boundaries are preserved. Sixteen authored classic/reordered/fragmented/hybrid movies,with and without silence/repeated edits,pass full PCM/video and raw reseek before/after;all16 fail old direct/actual-worker precise seeks and pass unchanged after. Every native timestamp and PCM byte is compared against independently sliced natural playback on exact8k/16k clocks. Seven gates include strict constant-edit packet stamps and nine PCM geometry normalization tests,passing1061 test executions,two old ignores;three repeats pass119 tests each. AAC/ALAC mixed-family routing remains outside this proof;no new ID,speedup or hardware claim.';p.write_text(json.dumps(r,indent=2)+'\n')
(q/'progress.json').write_text(json.dumps(dict(status='accepted',count_delta=0,follow_up_id='FVID-0484',accepted_count=511,next_id='FVID-0512',qualification_test_executions=1061,repeat_passed_each=119),indent=2)+'\n')
p=Path('docs/improvements/README.md');s=p.read_text();assert 'Accepted: **511 / 1000'in s
if '## Batch 520'not in s:
 s+='\n## Batch 520 — Mixed PCM/MACE/ADPCM native seek precision,zero new IDs\n\nFVID-0484 now preserves exact Duration floors across supported PCM/MACE/ADPCM description mixtures. Sixteen authored classic/reordered/fragmented/hybrid files cover normal playback,silence/repeated edits,native PCM suffixes/actual packet timestamps,raw reseek and actual-worker anchors. Seven gates pass1061 test executions,two prior ignores;three repeated119-test runs pass. Counter511,next FVID-0512. See `batch-520-validation.json`.\n';p.write_text(s)
p=q/'README.md';s=p.read_text().replace('Pending audit,zero accepted IDs','Accepted zero-count FVID-0484 follow-up').replace('Seven broader gates,including strict constant-edit packet stamps and nine PCM normalization tests,and three repeats are pending.','All seven broader gates include strict constant-edit packet stamps and nine PCM normalization tests,and pass1061 test executions,two prior ignores;three repeated119-test runs pass on frozen source.');p.write_text(s)
print('Accepted zero-count FVID-0484 mixed-native follow-up;511/1000')
