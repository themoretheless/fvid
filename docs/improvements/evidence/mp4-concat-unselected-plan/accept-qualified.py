"""Accept three independent corrections only from the frozen successful batch."""
from pathlib import Path
import json,hashlib,subprocess
q=Path(__file__).resolve().parent
results=json.loads((q/'qualification-results.json').read_text())
assert len(results)==5 and all(v['exit_code']==0 and v['failed']==0 for v in results.values())
assert (results['integration']['passed'],results['root-mp4']['passed'],results['default-focused']['passed'])==(119,83,12)
assert json.loads((q/'ignored-tests-comparison.json').read_text())['unchanged']
for n,h in json.loads((q/'batch-source.json').read_text()).items():assert hashlib.sha256(Path(n).read_bytes()).hexdigest()==h,n
for n,h in json.loads((q/'fixture-verification.json').read_text())['files'].items():assert hashlib.sha256(Path(n).read_bytes()).hexdigest()==h,n
assert not subprocess.check_output(['git','diff','--name-only','--','tests/*.rs'])
for n in ['crates/fvid-media/src/owned_probe.rs','crates/fvid-media/src/owned_aac/adts_impl.rs']:
 assert Path(n).read_bytes()==subprocess.check_output(['git','show','HEAD:'+n]),n
for name in ['mp4-concat-unselected-plan','audio-metadata-selection-window','audio-concat-plan-dispatch']:
 d=q.parent/name;assert json.loads((d/'test-integrity.json').read_text())['existing_rust_regression_files_modified']==False
 for n,h in json.loads((d/'after-source.json').read_text()).items():assert hashlib.sha256(Path(n).read_bytes()).hexdigest()==h,n
reg=Path('docs/improvements/registry.json');j=json.loads(reg.read_text());assert j['accepted_count']==473 and j['next_id']=='FVID-0474'
entries=[
 ('FVID-0474','Do not let excluded MP4 transport clocks reject selected video concat','src/container/mp4_concat.rs','tests/mp4_concat_unselected_plan.rs',
 'Three valid authored AVC/AAC movies decode correctly. Coarse1000/1001Hz AAC cannot use the compressed AAC transport planner;that excluded plan aborts selected-video concat and public plan/publication. Final five-case baseline:two controls pass,three failures including the new writer guard whose setup is blocked by the same admission failure.',
 'Preserve successful transport plans and retained-plan memory accounting,ignore unsupported plans only for excluded tracks,and always propagate cancellation. A private mask rejects selecting an unavailable plan before output. Five unchanged tests verify exact source YUV/AAC PCM,selected-AAC refusal,native80ms video playback and exact RGB/seek/rewind/EOF,public publication and one-packet prefix. Existing successful APIs and parser validation remain. No performance claim.'),
 ('FVID-0475','Separate audio metadata discovery from active encoded-window quotas','src/media_probe.rs','tests/audio_metadata_selection_window.rs',
 'Final identical four-case baseline passes source decode/seek and active-budget/cancel/memory refusal controls,but metadata copying blocks Opus/Vorbis trim to MKA under valid6-byte active limits,and an exhausted concat prefix cannot inspect later Ogg headers. Call traces identify metadata-only consumers using whole-source packet caps.',
 'Validate original cancellation,zero,RSS and memory controls before a private metadata-only probe. Preserve original encoded byte/work caps for codec/interval admission. Use the helper for standalone PCM metadata,audio-source discovery and generic stream selection. Four unchanged tests verify automatic/explicit trim,WAVE/MKA output,exact frames and bounded finite silence,Opus seek,an exact nonzero PCM concat prefix,and active oversized/zero/resource/cancellation refusals without output. Full-source public probe policy remains unchanged.'),
 ('FVID-0476','Use configured decoder dispatch when reporting admitted audio concat streams','src/native_plan.rs','tests/audio_concat_plan_dispatch.rs',
 'After successful concat admission,the legacy AAC/PCM planner parses G.711 RIFF as ADTS. Published-source G.711 controls fail in default and64MiB modes. With the preceding metadata correction staged,all six G.711/Opus/Vorbis plan variants fail as invalid ADTS;two source/resource controls pass.',
 'Report concat streams through the existing public decoder planner,retaining its codec routing and controls. Three unchanged tests verify G.711 source PCM against an independent scalar oracle,correct codec/disposition,publication of both complete segments across three formats and two memory modes,and invalid-selection/zero/resource refusals. G.711 concatenation retains every exact float sample;Opus/Vorbis retain exact frame counts and bounded finite numerical silence. No new codec or public API signature.')]
for id,title,source,test,before,result in entries:
 j['entries'].append(dict(id=id,batch=474,title=title,source=source,regression=test,before=before,result=result,status='accepted',validation='batch-474-validation.json'))
for e in j['entries']:
 if e['id']in['FVID-0257','FVID-0258']:
  e.setdefault('additional_validations',[]).append('batch-474-validation.json')
j['accepted_count']=476;j['next_id']='FVID-0477';reg.write_text(json.dumps(j,indent=2)+'\n')
b=dict(status='accepted',accepted_ids=[e[0]for e in entries],count_delta=3,accepted_count=476,next_id='FVID-0477',requalified_ids=['FVID-0257','FVID-0258'],stand_repair_count_delta=0,qualification=results,integration_passed=119,root_mp4_unit_executions_passed=83,default_feature_reexecutions_passed=12,default_reexecutions_excluded_from_distinct_claims=True,physical_source_distinct_count_not_claimed=True,old_root_mp4_ignore_unchanged=True,source_hashes='evidence/mp4-concat-unselected-plan/batch-source.json',fixture_hashes='evidence/mp4-concat-unselected-plan/fixture-verification.json',evidence=['evidence/'+n+'/README.md'for n in ['mp4-concat-unselected-plan','audio-metadata-selection-window','audio-concat-plan-dispatch']],limitations=['Native software video/codec playback,seek and actual audio export;no physical audio/GPU or new native-window duration qualification.','No performance claim.','Successful excluded MP4 plans remain retained and charged;no unused-plan memory optimization is claimed.'])
Path('docs/improvements/batch-474-validation.json').write_text(json.dumps(b,indent=2)+'\n')
for id,name in [('FVID-0474','mp4-concat-unselected-plan'),('FVID-0475','audio-metadata-selection-window'),('FVID-0476','audio-concat-plan-dispatch')]:
 d=q.parent/name;p=d/'README.md';p.write_text(p.read_text().replace('Candidate '+id+'.','Accepted '+id+' after the recorded frozen-source qualification.'))
 (d/'progress.json').write_text(json.dumps(dict(status='qualified;pending delivery',accepted_count=476,next_id='FVID-0477',accepted_ids=[id],session=None,goal_complete=False),indent=2)+'\n')
print('Accepted FVID-0474..0476;476/1000;stand repairs add0')
