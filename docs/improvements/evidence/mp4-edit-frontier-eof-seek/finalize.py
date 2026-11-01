import json,re,hashlib
from pathlib import Path
q=Path(__file__).parent
results=json.loads((q/'qualification-results.json').read_text());assert len(results)==7 and all(r['exit_code']==0 for r in results.values())
for name,r in results.items():
    rows=re.findall(r'test result: ok\. (\d+) passed; (\d+) failed; (\d+) ignored',(q/(name+'.log')).read_text())
    r['totals']={key:sum(int(row[i])for row in rows)for i,key in enumerate(['passed','failed','ignored'])};r['log']=str(q/(name+'.log'))
assert sum(r['totals']['passed']for r in results.values())==980
assert sum(r['totals']['ignored']for r in results.values())==2
repeats=json.loads((q/'repeat-results.json').read_text());assert repeats['exit_codes']==[0,0,0]
for i in range(1,4):
    rows=re.findall(r'test result: ok\. (\d+) passed; (\d+) failed; (\d+) ignored',(q/f'repeat-{i}.log').read_text());assert [sum(int(r[j])for r in rows)for j in range(3)]==[38,0,0]
frozen=json.loads((q/'qualification-source.json').read_text());assert all(hashlib.sha256(Path(p).read_bytes()).hexdigest()==h for p,h in frozen.items())
v=dict(status='accepted',accepted_ids=['FVID-0511'],count_delta=1,accepted_count=511,next_id='FVID-0512',qualification=results,repeat_runs=3,repeat_passed_each=38,source_frozen=frozen,before=dict(exit_code=101,passed=2,failed=3,log=str(q/'before.log')),existing_tests_unchanged=True,old_ignores=2,limitations=['No performance or physical audio-device claim.','This does not complete the MP4 timing audit.','Six fixture variants and three failing navigation checks represent one endpoint-bounds cause.'])
Path('docs/improvements/batch-512-validation.json').write_text(json.dumps(v,indent=2)+'\n')
p=Path('docs/improvements/registry.json');r=json.loads(p.read_text());assert r['accepted_count']==510 and r['next_id']=='FVID-0511'
r['entries'].append(dict(id='FVID-0511',batch=512,title='Bound occupied final-edit seek frontiers at the declared MP4 presentation end',source='src/playback/audio/mp4_audio.rs',regression='tests/mp4_edit_frontier_eof_seek.rs',before='Six authored AAC/PCM movies pass independent full PCM/video and raw-grid controls,but exact and actual-worker seeks in fully occupied final edits anchor beyond duration. AAC1020833ns exceeds1000020/1000100ns EOF;PCM1125000ns exceeds1100000ns. Raw seeks return14406/54 instead of bounded14113/52 ticks. Final baseline passes2/fails3.',result='Clamp the native-alignment loop and raw-variable minimum frontier at the declared presentation end before exposing seek clocks. Preserve complete final samples,raw tick units and FVID-0510 segment traversal. Seven frozen-source gates pass980 executions,two prior ignores;three repeats pass38 tests each.',status='accepted',validation='batch-512-validation.json'))
r['accepted_count']=511;r['next_id']='FVID-0512';p.write_text(json.dumps(r,indent=2)+'\n')
(q/'progress.json').write_text(json.dumps(dict(status='accepted',count_delta=1,accepted_ids=['FVID-0511'],accepted_count=511,next_id='FVID-0512',qualification_test_executions=980,repeat_passed_each=38),indent=2)+'\n')
p=Path('docs/improvements/README.md');s=p.read_text();assert 'Accepted: **510 / 1000'in s;s=s.replace('Accepted: **510 / 1000','Accepted: **511 / 1000',1);s+='\n## Batch 512 — Bound occupied final edit seek frontiers at declared EOF\n\nFVID-0511 prevents exact and raw MP4 seek clocks from escaping the declared presentation endpoint when a preceding native sample occupies every remaining edit. Six authored AAC/PCM+VP9 movies retain full independent PCM/video while direct and actual-worker EOF anchors now stay bounded. Raw navigation keeps its media-tick contract. Seven gates pass980 test executions,two prior ignored tests,and three repeated38-test runs pass. See `batch-512-validation.json` and `evidence/mp4-edit-frontier-eof-seek`.\n';p.write_text(s)
print('Accepted FVID-0511;511/1000')
