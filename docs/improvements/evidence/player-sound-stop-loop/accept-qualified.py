"""Accept only the unchanged fully qualified sound stop ordering fix."""
from pathlib import Path
import hashlib,json,sys
q=Path(__file__).resolve().parent
checks=json.loads((q/'qualification-results.json').read_text())
assert len(checks)==9 and all(v['exit_code']==0 and v['failed']==0 for v in checks.values())
for v in checks.values():assert hashlib.sha256((q/v['archive']).read_bytes()).hexdigest()==v['archived_sha256']
for n,h in json.loads((q/'after-source.json').read_text()).items():assert hashlib.sha256(Path(n).read_bytes()).hexdigest()==h,n
rep=json.loads((q/'repeat.json').read_text());assert len(rep['runs'])==20 and rep['total_case_executions']==100
assert all(r['exit_code']==0 and r['passed']==5 for r in rep['runs'])
d=json.loads((q/'deduplication.json').read_text());assert d['distinct_passed']==1581
assert d['ignored_by_gate']==json.loads((q.parent/'player-io-clock/deduplication.json').read_text())['ignored_by_gate']
sys.dont_write_bytecode=True;sys.path.insert(0,'scripts');from generate_player_sound_stop_loop import generate
for n,b in generate().items():assert (Path('tests/fixtures/playback-errors/player-sound-stop-loop')/n).read_bytes()==b
before=json.loads((q/'before-source.json').read_text());after=json.loads((q/'after-source.json').read_text())
t='src/player_sound_stop_loop_tests.rs';assert before[t]==after[t]
reg=Path('docs/improvements/registry.json');j=json.loads(reg.read_text());assert j['accepted_count']==466 and j['next_id']=='FVID-0467'
j['entries'].append(dict(id='FVID-0467',batch=467,title='Spend an explicit sound-only stop before issuing an A-B rewind',source='src/player.rs',regression=t,before='Identical five-case baseline passes exact video/PCM seek/EOF,unreached-stop looping and paused controls,but both earlier75ms and coincident100ms stop cases send an unwanted seek to A25ms at150ms callback. Real audio worker generation becomes1 instead of0. This sound-only path is byte-identical before0463/0464.',result='Check explicit stop before the sound-only loop and return once consumed. Actual producer pauses without a seek,terminal phase parks at the requested bound,and subsequent explicit navigation resumes to natural EOF. Five unchanged regressions pass20repetitions,100case executions. All9gates pass1581physical-source-distinct checks and both builds;five old ignores unchanged. No performance,physical audio or new native-window duration claim.',status='accepted',validation='batch-467-validation.json'))
j['accepted_count']=467;j['next_id']='FVID-0468';reg.write_text(json.dumps(j,indent=2)+'\n')
b=dict(status='accepted',accepted_ids=['FVID-0467'],count_delta=1,accepted_count=467,before_commit=(q/'base.txt').read_text().strip(),baseline=dict(passed=3,failed=2,evidence='evidence/player-sound-stop-loop/before.log'),focused_after=dict(passed=5,failed=0,evidence='evidence/player-sound-stop-loop/after-focused.log'),repetition=dict(runs=20,case_executions=100,failed=0,excluded_from_distinct_and_improvement_counts=True),checks=checks,physical_source_distinct_passed=1581,ignored_tests_unchanged=True,test_integrity='evidence/player-sound-stop-loop/test-integrity.json',evidence='evidence/player-sound-stop-loop/README.md',limitations=['Real native decoding/audio worker and headless egui controller with controlled output clock;no physical audio.','No new native-window duration qualification;pre-existing viewport-close-request limitation remains at evidence/player-ab-clock/native-smoke/finding.json.','No performance or hardware-codec claim.'])
Path('docs/improvements/batch-467-validation.json').write_text(json.dumps(b,indent=2)+'\n')
p=q/'README.md';s=p.read_text();s=s.replace('Unaccepted audit until the identical baseline, candidate and all subsystem gates are qualified. Proposed next ID FVID-0467, one ordering problem.','Accepted FVID-0467 after the identical baseline/candidate and all subsystem gates were qualified. One ordering problem.');s+='\nBaseline:3pass2fail,both failures prove an actual unwanted audio seek(generation1 instead of0),not a source-admission error. Candidate:5pass0fail on the unchanged test hash. Twenty repeats pass100case executions. Nine gates and both builds pass1581distinct tests;the same five old ignores remain. The old pre0463 tick_sound body is byte-identical to the baseline(classification.json). No other improvement IDs counted.\n';p.write_text(s)
i=json.loads((q/'test-integrity.json').read_text());i['focused_candidate']=dict(passed=5,failed=0,exit_code=0);(q/'test-integrity.json').write_text(json.dumps(i,indent=2)+'\n')
(q/'progress.json').write_text(json.dumps(dict(status='qualified;pending delivery',accepted_count=467,next_id='FVID-0468',accepted_ids=['FVID-0467'],active_session=None),indent=2)+'\n')
print('Accepted FVID-0467,467/1000;all gates,source and fixtures verified')
