import json,re,hashlib
from pathlib import Path
q=Path(__file__).parent
results=json.loads((q/'qualification-results.json').read_text());assert len(results)==7 and all(r['exit_code']==0 for r in results.values())
for name,r in results.items():
    rows=re.findall(r'test result: ok\. (\d+) passed; (\d+) failed; (\d+) ignored',(q/(name+'.log')).read_text())
    r['totals']={key:sum(int(row[i])for row in rows)for i,key in enumerate(['passed','failed','ignored'])};r['log']=str(q/(name+'.log'))
assert sum(r['totals']['passed']for r in results.values())==975
assert sum(r['totals']['ignored']for r in results.values())==2
repeats=json.loads((q/'repeat-results.json').read_text());assert repeats['exit_codes']==[0,0,0]
for i in range(1,4):
    rows=re.findall(r'test result: ok\. (\d+) passed; (\d+) failed; (\d+) ignored',(q/f'repeat-{i}.log').read_text());assert [sum(int(r[j])for r in rows)for j in range(3)]==[33,0,0]
frozen=json.loads((q/'qualification-source.json').read_text());assert all(hashlib.sha256(Path(p).read_bytes()).hexdigest()==h for p,h in frozen.items())
v=dict(status='accepted',accepted_ids=['FVID-0510'],count_delta=1,accepted_count=510,next_id='FVID-0511',qualification=results,repeat_runs=3,repeat_passed_each=33,source_frozen=frozen,before=dict(exit_code=101,passed=2,failed=2,log=str(q/'before.log')),existing_tests_unchanged=True,old_ignores=2,limitations=['No performance or physical audio-device claim.','This is not certification of every MP4 timing or mixed codec-family path.'])
v['merge_integration_repair']=dict(count_delta=0,before_log=str(q/'merge-initial-qualification/media-units.log'),before_passed=546,before_failed=2,focused_passed=22,tests_unchanged=True,patch=str(q/'merge-repair.patch'))
Path('docs/improvements/batch-511-validation.json').write_text(json.dumps(v,indent=2)+'\n')
p=Path('docs/improvements/registry.json');r=json.loads(p.read_text());assert r['accepted_count']==509 and r['next_id']=='FVID-0511' and r['entries'][-1]['id']=='FVID-0510' and r['entries'][-1]['status']=='pending'
r['entries'][-1]=dict(id='FVID-0510',batch=511,title='Reapply occupied edit frontiers when native-sample seeking traverses segments',source='src/playback/audio/mp4_audio.rs',regression='tests/mp4_edit_frontier_seek.rs',before='Three authored LC48k/Main32k movies pass full independent PCM/video and raw tick controls,but exact seek at1000001ns or1000009ns crosses the first edit and anchors at1000010ns rather than the first available sample1031260ns. Both direct and actual-worker regressions fail despite correct suffix PCM.',result='Apply the newly selected segment minimum occupied-sample start on every alignment-loop traversal before deriving its native anchor. Original Duration fractions are already preserved;this fixes a distinct missed frontier after segment crossing. Seven gates pass975 executions,two prior ignores;three repeats pass33 tests each.',status='accepted',validation='batch-511-validation.json')
r['accepted_count']=510;r['next_id']='FVID-0511';p.write_text(json.dumps(r,indent=2)+'\n')
(q/'progress.json').write_text(json.dumps(dict(status='accepted',count_delta=1,accepted_ids=['FVID-0510'],accepted_count=510,next_id='FVID-0511',qualification_test_executions=975,repeat_passed_each=33),indent=2)+'\n')
print('Accepted FVID-0510;510/1000')

p=Path('docs/improvements/README.md');s=p.read_text();assert 'Accepted: **509 / 1000'in s;s=s.replace('Accepted: **509 / 1000','Accepted: **510 / 1000',1);s+='\n## Batch 511 — Preserve native seek anchors across occupied edit boundaries\n\nFVID-0510 reapplies each selected edit segment minimum occupied-sample start after native-sample rounding crosses a segment boundary. Three authored AAC+VP9 movies reproduce correct PCM with an early clock anchor before the fix;direct navigation and actual AudioWorker flush anchors now match the first available native sample. Seven merged-state gates pass975 test executions,with two prior ignored tests,and three repeated33-test runs pass. The ADTS initial transport-admission integration repair restores two unchanged unit tests and adds zero IDs. Parallel entries through FVID-0509 are preserved. See `batch-511-validation.json` and `evidence/mp4-edit-frontier-seek`.\n';p.write_text(s)
