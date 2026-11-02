import json,re,hashlib
from pathlib import Path
q=Path(__file__).parent
results=json.loads((q/'qualification-results.json').read_text());assert len(results)==7 and all(r['exit_code']==0 for r in results.values())
for name,r in results.items():
    rows=re.findall(r'test result: ok\. (\d+) passed; (\d+) failed; (\d+) ignored',(q/(name+'.log')).read_text())
    r['totals']={key:sum(int(row[i])for row in rows)for i,key in enumerate(['passed','failed','ignored'])};r['log']=str(q/(name+'.log'))
assert sum(r['totals']['passed']for r in results.values())==985
assert sum(r['totals']['ignored']for r in results.values())==2
repeats=json.loads((q/'repeat-results.json').read_text());assert repeats['exit_codes']==[0,0,0]
for i in range(1,4):
    rows=re.findall(r'test result: ok\. (\d+) passed; (\d+) failed; (\d+) ignored',(q/f'repeat-{i}.log').read_text());assert [sum(int(r[j])for r in rows)for j in range(3)]==[43,0,0]
frozen=json.loads((q/'qualification-source.json').read_text());assert all(hashlib.sha256(Path(p).read_bytes()).hexdigest()==h for p,h in frozen.items())
fixtures=json.loads((q/'fixture-verification.json').read_text());assert all(hashlib.sha256((Path('tests/fixtures/playback-errors/mp4-unedited-pcm-subtick-seek')/n).read_bytes()).hexdigest()==h for n,h in fixtures.items())
baseline=json.loads((q/'player-baseline-result.json').read_text());assert baseline['exit_code']==101 and baseline['candidate_restored']
player=json.loads((q/'player-candidate-results.json').read_text());assert len(player)==2 and all(v['exit_code']==0 for v in player.values())
assert 'test result: ok. 7 passed; 0 failed;' in(q/'player-after.log').read_text()
v=dict(status='accepted',accepted_ids=[],follow_up_ids=['FVID-0484'],count_delta=0,accepted_count=511,next_id='FVID-0512',qualification=results,repeat_runs=3,repeat_passed_each=43,source_frozen=frozen,before=dict(exit_code=101,passed=3,failed=2,log=str(q/'before.log')),existing_tests_unchanged=False,new_regressions_unchanged_before_after=True,existing_test_expectation_migration=dict(test="acknowledged_fractional_request_releases_to_quantized_actual_mp4_clock",old_sample=600,new_sample=601,original_new_expectation_exit=101,patch=str(q/"player-expectation.patch"),timeouts_and_tolerances_unchanged=True),old_ignores=2,limitations=['No performance or physical audio-device claim.','Other unedited non-PCM codec families retain their existing paths.','Same initial Duration-to-media-tick loss as FVID-0484;zero new IDs.'])
Path('docs/improvements/batch-513-validation.json').write_text(json.dumps(v,indent=2)+'\n')
p=Path('docs/improvements/registry.json');r=json.loads(p.read_text());assert r['accepted_count']==511 and r['next_id']=='FVID-0512'
e=next(e for e in r['entries']if e['id']=='FVID-0484');e['unedited_pcm_follow_up_validation']='batch-513-validation.json';e['unedited_pcm_follow_up_note']='Zero-count unedited native PCM extension. Retain Duration fractions through per-packet clipping and anchor at the selected native rate. Eleven authored classic/fragmented/compact/reordered movies cover four media clocks,alias/endian headers and48k stereo16/32k mono float32/44.1k stereo24 transitions. Full independent PCM/video,raw tick compatibility and raw reseek controls pass before/after;two precision regressions fail then pass. Seven frozen-source gates pass985 executions,two prior ignores;three repeats pass43 tests each. No performance or physical audio claim.';p.write_text(json.dumps(r,indent=2)+'\n')
(q/'progress.json').write_text(json.dumps(dict(status='accepted',count_delta=0,follow_up_id='FVID-0484',accepted_count=511,next_id='FVID-0512',qualification_test_executions=985,repeat_passed_each=43),indent=2)+'\n')
print('Accepted zero-count FVID-0484 follow-up;511/1000')

p=Path('docs/improvements/README.md');s=p.read_text();assert 'Accepted: **511 / 1000'in s
if '## Batch 513'not in s:
 s+='\n## Batch 513 — Unedited native PCM seek precision,zero new IDs\n\nFVID-0484 now preserves exact Duration floors for unedited native PCM,including compact indexes and changes in rate,channels and storage format. Eleven authored movies retain independent full PCM/video,precise suffixes,raw navigation and actual-worker anchors. The fractional Player test checks native sample601 instead of the previously repeated sample600;the original source fails that stricter expectation,and all other player assertions remain. Seven gates pass985 executions,two prior ignores,and three repeated43-test runs pass. Counter511,next FVID-0512. See `batch-513-validation.json`.\n';p.write_text(s)
