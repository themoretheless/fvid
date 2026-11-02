import json,re,hashlib
from pathlib import Path
q=Path(__file__).parent
results=json.loads((q/'qualification-results.json').read_text());assert len(results)==7 and all(r['exit_code']==0 for r in results.values())
for name,r in results.items():
    rows=re.findall(r'test result: ok\. (\d+) passed; (\d+) failed; (\d+) ignored',(q/(name+'.log')).read_text())
    r['totals']={key:sum(int(row[i])for row in rows)for i,key in enumerate(['passed','failed','ignored'])};r['log']=str(q/(name+'.log'))
assert sum(r['totals']['passed']for r in results.values())==1000
assert sum(r['totals']['ignored']for r in results.values())==2
repeats=json.loads((q/'repeat-results.json').read_text());assert repeats['exit_codes']==[0,0,0]
for i in range(1,4):
    rows=re.findall(r'test result: ok\. (\d+) passed; (\d+) failed; (\d+) ignored',(q/f'repeat-{i}.log').read_text());assert [sum(int(r[j])for r in rows)for j in range(3)]==[58,0,0]
frozen=json.loads((q/'qualification-source.json').read_text());assert all(hashlib.sha256(Path(p).read_bytes()).hexdigest()==h for p,h in frozen.items())
fixtures=json.loads((q/'fixture-verification.json').read_text())['sha256'];assert len(fixtures)==33 and all(hashlib.sha256((Path('tests/fixtures/playback-errors/mp4-adpcm-subtick-seek')/n).read_bytes()).hexdigest()==h for n,h in fixtures.items())
v=dict(status='accepted',accepted_ids=[],follow_up_ids=['FVID-0484'],count_delta=0,accepted_count=511,next_id='FVID-0512',qualification=results,repeat_runs=3,repeat_passed_each=58,source_frozen=frozen,before=dict(exit_code=101,passed=3,failed=2,log=str(q/'before.log')),existing_tests_unchanged=True,old_ignores=2,refusal=dict(file='tests/fixtures/playback-errors/mp4-adpcm-subtick-seek/invalid-ms-predictor.mp4',original_passed=True,final_passed=True,expected_error='MS ADPCM predictor index exceeds coefficient table',targets=['16250000ns','16250001ns']),limitations=['No speedup or physical audio-device claim.','Mixed codec-family timelines retain their prior paths.','Same initial Duration-to-media-tick loss as FVID-0484;zero new IDs.','Sixteen valid variants and the active refusal control share one existing floor-loss cause.'])
Path('docs/improvements/batch-516-validation.json').write_text(json.dumps(v,indent=2)+'\n')
p=Path('docs/improvements/registry.json');r=json.loads(p.read_text());assert r['accepted_count']==511 and r['next_id']=='FVID-0512'
e=next(e for e in r['entries']if e['id']=='FVID-0484');e['adpcm_follow_up_validation']='batch-516-validation.json';e['adpcm_follow_up_note']='Zero-count ADPCM extension. Pure IMA4/IMA-WAV/MS-ADPCM descriptions retain exact Duration floors and derive native frame counts from coded block geometry. Sixteen authored movies verify full bit-exact PCM/video,grouped blocks,three clocks,fragmented/reordered/trimmed tracks,changing rates,silence,repeated edits,precise suffixes and actual-worker anchors. Invalid active MS predictor refuses before/after. Seven gates pass1000 test executions,two old ignores;three repeats pass58 tests each. No speedup or hardware claim.';p.write_text(json.dumps(r,indent=2)+'\n')
(q/'progress.json').write_text(json.dumps(dict(status='accepted',count_delta=0,follow_up_id='FVID-0484',accepted_count=511,next_id='FVID-0512',qualification_test_executions=1000,repeat_passed_each=58),indent=2)+'\n')
p=Path('docs/improvements/README.md');s=p.read_text();assert 'Accepted: **511 / 1000'in s
if '## Batch 516'not in s:
    s+='\n## Batch 516 — ADPCM native seek precision,zero new IDs\n\nFVID-0484 now retains exact Duration floors in pure ADPCM timelines. Sixteen authored movies verify bit-exact PCM/video,grouped blocks,rate changes,silence,repeated edits and actual-worker anchors. Invalid active MS predictors still refuse. Seven gates pass1000 test executions,two old ignores,and three repeated58-test runs pass. Counter511,next FVID-0512. See `batch-516-validation.json`.\n';p.write_text(s)
print('Accepted zero-count FVID-0484 ADPCM follow-up;511/1000')
