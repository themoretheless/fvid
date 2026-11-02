import json,re,hashlib
from pathlib import Path
q=Path(__file__).parent
results=json.loads((q/'qualification-results.json').read_text());assert len(results)==7 and all(r['exit_code']==0 for r in results.values())
def totals(p):
 rows=re.findall(r'test result: ok\. (\d+) passed; (\d+) failed; (\d+) ignored',p.read_text())
 return {k:sum(int(row[i])for row in rows)for i,k in enumerate(['passed','failed','ignored'])}
for n,r in results.items():r.update(totals=totals(q/(n+'.log')),log=str(q/(n+'.log')))
assert sum(r['totals']['passed']for r in results.values())==1068
assert sum(r['totals']['failed']for r in results.values())==0
assert sum(r['totals']['ignored']for r in results.values())==2
repeats=json.loads((q/'repeat-results.json').read_text());assert repeats['exit_codes']==[0,0,0]
for i in range(1,4):assert totals(q/f'repeat-{i}.log')==dict(passed=126,failed=0,ignored=0)
assert totals(q/'after-focused.log')==dict(passed=7,failed=0,ignored=0)
frozen=json.loads((q/'qualification-source.json').read_text())
for p,h in frozen.items():assert hashlib.sha256(Path(p).read_bytes()).hexdigest()==h,p
fixtures=json.loads((q/'fixture-verification.json').read_text());assert len(fixtures)==19
for n,h in fixtures.items():assert hashlib.sha256((Path('tests/fixtures/playback-errors/mp4-mixed-coded-seek')/n).read_bytes()).hexdigest()==h,n
assert 'test result: FAILED. 3 passed; 2 failed;'in(q/'before.log').read_text()
assert 'test result: FAILED. 1 passed; 1 failed;'in(q/'before.log').read_text()
assert 'test result: FAILED. 3 passed; 2 failed;'in(q/'after-phase-only.log').read_text()
assert 'test result: ok. 2 passed; 0 failed;'in(q/'after-phase-only.log').read_text()
independence=json.loads((q/'independent-causes.json').read_text());assert len(independence['duration_route_still_fails_after_stamp_only']['cases'])==16
phase=json.loads((q/'phase-only-source.json').read_text())['sha256']['src/playback/audio/mp4_audio.rs'];assert hashlib.sha256((q/'phase-only-source.rs.txt').read_bytes()).hexdigest()==phase
assert json.loads((q/'test-integrity.json').read_text())['new_tests_helpers_generator_identical_before_after']
v=dict(status='accepted',accepted_ids=['FVID-0512'],follow_up_ids=['FVID-0484'],count_delta=1,accepted_count=512,next_id='FVID-0513',qualification=results,repeat_runs=3,repeat_passed_each=126,source_frozen=frozen,before=dict(exit_code=101,mixed_passed=3,mixed_failed=2,raw_passed=1,raw_failed=1,log=str(q/'before.log')),after_stamp_only=dict(raw_passed=2,raw_failed=0,mixed_passed=3,mixed_failed=2,log=str(q/'after-phase-only.log')),independence=independence,existing_tests_unchanged=True,old_ignores=2,fixture_stand_repairs_counted=0,limitations=['No performance or physical audio-device claim.','Exact packet fractions are preserved when representable;existing stamp-helper integer-width fallback remains unchanged.','Sixteen mixed LC1024/Main960 AAC and ALAC raw/Rice witnesses qualify specified48/32/44.1k profiles,not every possible codec profile.','The natural-playback oracle qualifies seek history;the separate pure PCM fixture also has an independent arithmetic waveform.'])
vp=Path('docs/improvements/batch-521-validation.json')
if vp.exists():
 existing=json.loads(vp.read_text());assert existing.get('accepted_ids')==['FVID-0512'] and existing.get('source_frozen',{}).get('tests/mp4_native_pcm_fractional_stamp.rs')==frozen['tests/mp4_native_pcm_fractional_stamp.rs'],'Not overwriting a foreign validation'
p=Path('docs/improvements/registry.json');registry_before=p.read_bytes();r=json.loads(registry_before);found=next((e for e in r['entries']if e['id']=='FVID-0512'),None)
entry=dict(id='FVID-0512',batch=521,title='Preserve representable exact native PCM packet timestamps after raw media-tick clipping',source='src/playback/audio/mp4_audio.rs',regression='tests/mp4_native_pcm_fractional_stamp.rs',before='An authored pure44.1k stereo24-bit80-frame PCM movie passes exact arithmetic waveform and six-picture controls. Raw media-tick seek,without Duration conversion,returns the right suffix but labels native sample1 as22675/1e9 seconds;floor(timestamp*44100) is0 instead of1. Original raw suite passes1/fails1. A stamp-only correction passes both raw tests while mixed Duration tests still fail3pass2fail on16 cases,proving independence from FVID-0484.',result='Use integer nanoseconds only when the native fraction is exactly integral and fits u64;otherwise use the existing reduced packet-stamp helper. Preserve PCM bytes,raw tick/packet landing contracts,integer-nanosecond formatting when exact and the old unrepresentable-width fallback. Seven frozen-source gates pass1068 test executions,two prior ignores;three repeats pass126 tests each. Independent-cause evidence and the complete measured stamp-only source are retained.',status='accepted',validation='batch-521-validation.json')
if found is None:
 assert r['accepted_count']==511 and r['next_id']=='FVID-0512';r['entries'].append(entry)
else:assert found==entry,'ID occupied by another change'
r['accepted_count']=sum(e['status']=='accepted'for e in r['entries']);r['next_id']=f"FVID-{len(r['entries'])+1:04d}";assert r['accepted_count']==512 and r['next_id']=='FVID-0513'
e=next(e for e in r['entries']if e['id']=='FVID-0484');e['mixed_coded_follow_up_validation']='batch-521-validation.json';e['mixed_coded_follow_up_note']='Zero-count mixed AAC/ALAC/PCM routing extension. Native landing now dispatches ALAC coded counts by the actual selected packet through unchanged bounded header/fallback admission;AAC/ALAC descriptions join precise routing while constant AAC retains its origin-aware path. Sixteen authored classic/reordered/fragmented/hybrid files,with/without silence/repeated windows,verify complete native PCM/video,raw rational timestamps,decoder history,raw reseek and actual-worker anchors on48/32/44.1k. Mixed tests still fail after isolated FVID-0512 packet-stamp repair,then all unchanged focused tests pass after routing correction. Only FVID-0512 adds one independent ID;this extension and the generator-only mvhd stand repair add zero.';assert p.read_bytes()==registry_before,'Registry changed concurrently'
vp.write_text(json.dumps(v,indent=2)+'\n');p.write_text(json.dumps(r,indent=2)+'\n')
(q/'progress.json').write_text(json.dumps(dict(status='accepted',accepted_ids=['FVID-0512'],follow_up_ids=['FVID-0484'],count_delta=1,accepted_count=512,next_id='FVID-0513',qualification_test_executions=1068,repeat_passed_each=126),indent=2)+'\n')
p=Path('docs/improvements/README.md');s=p.read_text();assert 'Accepted: **511 / 1000'in s or 'Accepted: **512 / 1000'in s;s=s.replace('Accepted: **511 / 1000','Accepted: **512 / 1000')
if '## Batch 521'not in s:
 s+='\n## Batch 521 — Exact native packet stamps and mixed coded seek\n\nFVID-0512 preserves representable rational native PCM timestamps after raw media-tick clipping. A pure44.1k fixture proves the right waveform had the wrong native sample label,without any Duration request;the isolated stamp correction passes its regression while FVID-0484 mixed routing still fails. A separate zero-count extension then corrects AAC/ALAC/PCM routing and packet-specific ALAC counts. Sixteen mixed movies preserve rational timestamps,PCM,history,silence/repeated edits and actual-worker anchors. Seven gates pass1068 test executions,two prior ignores;three repeated126-test runs pass. Counter512,next FVID-0513. See `batch-521-validation.json`.\n';p.write_text(s)
p=q/'README.md';s=p.read_text().replace('Pending qualification;accepted counter511,next FVID-0512.','Accepted:one independent FVID-0512 plus a zero-count FVID-0484 follow-up;counter512,next FVID-0513.').replace('Candidate FVID-0512','FVID-0512').replace('Seven broader gates and three repeats remain pending.','All seven broader gates pass1068 test executions,two prior ignores;three repeated126-test runs pass on frozen source.');p.write_text(s)
print('Accepted FVID-0512;FVID-0484 follow-up zero count;512/1000')
