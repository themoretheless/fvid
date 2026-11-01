import json,re,hashlib
from pathlib import Path
q=Path(__file__).parent
results=json.loads((q/'qualification-results.json').read_text());assert len(results)==7 and all(r['exit_code']==0 for r in results.values())
for name,r in results.items():
 rows=re.findall(r'test result: ok\. (\d+) passed; (\d+) failed; (\d+) ignored',(q/(name+'.log')).read_text())
 r['totals']={key:sum(int(row[i])for row in rows)for i,key in enumerate(['passed','failed','ignored'])};r['log']=str(q/(name+'.log'))
assert sum(r['totals']['passed']for r in results.values())==876
assert sum(r['totals']['ignored']for r in results.values())==2
repeats=json.loads((q/'repeat-results.json').read_text());assert repeats['exit_codes']==[0,0,0]
for i in range(1,4):
 rows=re.findall(r'test result: ok\. (\d+) passed; (\d+) failed; (\d+) ignored',(q/f'repeat-{i}.log').read_text());assert [sum(int(r[j])for r in rows)for j in range(3)]==[29,0,0]
frozen=json.loads((q/'qualification-source.json').read_text());assert all(hashlib.sha256(Path(p).read_bytes()).hexdigest()==h for p,h in frozen.items())
v=dict(status='accepted',accepted_ids=[],follow_up_ids=['FVID-0484'],count_delta=0,accepted_count=485,next_id='FVID-0486',qualification=results,repeat_runs=3,repeat_passed_each=29,source_frozen=frozen,before=dict(exit_code=101,passed=3,failed=2,log=str(q/'before-final.log')),existing_tests_unchanged=True,old_ignores=2,limitations=['No performance or physical audio-device claim.','Mixed codec-family timelines retain their existing path.','Same lost-duration-fraction cause as FVID-0484; zero new IDs.'])
Path('docs/improvements/batch-486-validation.json').write_text(json.dumps(v,indent=2)+'\n')
p=Path('docs/improvements/registry.json');r=json.loads(p.read_text());assert r['accepted_count']==485 and r['next_id']=='FVID-0486'
e=next(e for e in r['entries']if e['id']=='FVID-0484');e['changing_aac_follow_up_validation']='batch-486-validation.json';e['changing_aac_follow_up_note']='Zero-count exact-grid changing AAC extension. Selected AAC descriptions use exact Duration floors despite differing rates,channels,LC/Main geometry or unused opaque catalog entries. Six valid authored movies retain independent PCM/video,whole-packet EOF,raw tick compatibility and actual-worker flush anchors;separate unavailable-source refusal passes before/after. Seven frozen-source gates pass876 executions,two prior ignores;three repeats pass29 tests each. No performance or physical audio claim.'
p.write_text(json.dumps(r,indent=2)+'\n')
(q/'progress.json').write_text(json.dumps(dict(status='accepted',count_delta=0,accepted_count=485,next_id='FVID-0486',qualification_test_executions=876,repeat_passed_each=29),indent=2)+'\n')
print('Accepted zero-count follow-up;485/1000')
