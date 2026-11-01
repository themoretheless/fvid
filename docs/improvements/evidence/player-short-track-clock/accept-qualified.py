"""Accept only the unchanged fully qualified replacement clock continuity fix."""
from pathlib import Path
import hashlib,json,sys
q=Path(__file__).resolve().parent
checks=json.loads((q/'qualification-results.json').read_text())
assert len(checks)==9 and all(v['exit_code']==0 and v['failed']==0 for v in checks.values())
for v in checks.values():assert hashlib.sha256((q/v['archive']).read_bytes()).hexdigest()==v['archived_sha256']
for n,h in json.loads((q/'after-source.json').read_text()).items():assert hashlib.sha256(Path(n).read_bytes()).hexdigest()==h,n
rep=json.loads((q/'repeat.json').read_text());assert len(rep['runs'])==20 and rep['total_case_executions']==180
assert all(r['exit_code']==0 and r['passed']==9 for r in rep['runs'])
d=json.loads((q/'deduplication.json').read_text());assert d['distinct_passed']==1632
assert d['ignored_by_gate']==json.loads((q.parent/'player-track-source-lifetime/deduplication.json').read_text())['ignored_by_gate']
sys.dont_write_bytecode=True;sys.path.insert(0,'scripts');from generate_player_short_track_clock import generate
for n,b in generate().items():assert (Path('tests/fixtures/playback-errors/player-short-track-clock')/n).read_bytes()==b
before=json.loads((q/'before-source.json').read_text());after=json.loads((q/'after-source.json').read_text())
t='src/player_short_track_clock_tests.rs';assert before[t]==after[t]
reg=Path('docs/improvements/registry.json');j=json.loads(reg.read_text());assert j['accepted_count']==472 and j['next_id']=='FVID-0473'
j['entries'].append(dict(id='FVID-0473',batch=473,title='Keep video time monotonic when a replacement audio track has already ended',source='src/player.rs',regression=t,before='Identical nine-case baseline passes exact unequal-track native video/PCM/EOF,unexpired replacement,explicit backward seek and sound-only completion controls,but five source-replacement transports fail. Physical599.875ms short-track endpoint rewinds750ms live/paused/silent video phase;stop timing is lost;immediate long-track return also requests599.875ms instead750ms.',result='Retain the published picture phase and raw source anchor during audio-clock replacement. A shorter endpoint cannot take time backward;real EOF/error hands time to the retained phase. Real catch-up,silent-picture ownership and explicit navigation release the guard. Subsequent track selection preserves the raw intent. Nine unchanged tests verify no fabricated past-end PCM,exact native recovery PCM,paused and silent handoff,775ms stop,backward450ms recovery and natural1500ms video EOF. Twenty repeats pass180cases;all9gates pass1632physical-source-distinct checks and both builds with five old ignores unchanged. No metadata-only EOF shortcut,performance,physical audio or new native-window duration claim.',status='accepted',validation='batch-473-validation.json'))
j['accepted_count']=473;j['next_id']='FVID-0474';reg.write_text(json.dumps(j,indent=2)+'\n')
b=dict(status='accepted',accepted_ids=['FVID-0473'],count_delta=1,accepted_count=473,before_commit=(q/'base.txt').read_text().strip(),baseline=dict(passed=4,failed=5,evidence='evidence/player-short-track-clock/before.log'),focused_after=dict(passed=9,failed=0,evidence='evidence/player-short-track-clock/after-focused.log'),repetition=dict(runs=20,case_executions=180,failed=0,excluded_from_distinct_and_improvement_counts=True),checks=checks,physical_source_distinct_passed=1632,ignored_tests_unchanged=True,test_integrity='evidence/player-short-track-clock/test-integrity.json',evidence='evidence/player-short-track-clock/README.md',limitations=['Real native decoding/audio worker and headless egui controller with controlled output clock;no physical audio.','No new native-window duration qualification;pre-existing viewport-close-request limitation remains at evidence/player-ab-clock/native-smoke/finding.json.','No performance or hardware-codec claim.'])
Path('docs/improvements/batch-473-validation.json').write_text(json.dumps(b,indent=2)+'\n')
p=q/'README.md';s=p.read_text().replace('Unaccepted audit.','Accepted FVID-0473 after all nine gates and twenty final repetitions passed.');s+='\nFinal identical nine-case baseline:4pass5fail,candidate:9pass0fail. Twenty repetitions pass180case executions. All9gates pass1632physical-source-distinct checks and both builds;five old ignores unchanged. The initial eight-case baseline and final expansion are preserved separately. Variant controls and additional synchronization add0IDs;no existing assertion is removed or relaxed.\n';p.write_text(s)
i=json.loads((q/'test-integrity.json').read_text());i['focused_candidate']=dict(passed=9,failed=0,exit_code=0);(q/'test-integrity.json').write_text(json.dumps(i,indent=2)+'\n')
(q/'progress.json').write_text(json.dumps(dict(status='qualified;pending delivery',accepted_count=473,next_id='FVID-0474',accepted_ids=['FVID-0473'],active_session=None),indent=2)+'\n')
print('Accepted FVID-0473,473/1000;all gates,source and fixtures verified')
