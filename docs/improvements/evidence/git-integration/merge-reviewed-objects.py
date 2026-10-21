import subprocess,json,os,tempfile
from pathlib import Path
ref='refs/heads/codex/integrate-preserved-branches-20261022'
base=subprocess.check_output(['git','rev-parse','HEAD']).decode().strip()
fd,idx=tempfile.mkstemp(prefix='fvid-integrate-index-');os.close(fd);os.unlink(idx)
env={**os.environ,'GIT_INDEX_FILE':idx}
def g(*a,input=None,check=True,index=False):return subprocess.run(['git','-c','gc.auto=0','-c','maintenance.auto=false',*a],input=input,stdout=subprocess.PIPE,stderr=subprocess.PIPE,check=check,env=env if index else None)
head=g('rev-parse','--verify',ref,check=False)
head=head.stdout.decode().strip() if head.returncode==0 else base
g('update-ref',ref,head)
reviewed={'crates/fvid-cpu/src/lib.rs','src/codec/hevc_decoder.rs','src/playback/video/webm.rs','src/player_gpu.rs','src/codec/hevc_transform.rs','src/codec/mod.rs','src/codec/vp9_picture.rs','tests/av1_chroma_grain.rs','crates/fvid-media/src/owned_aac/mod.rs','docs/CODEC_PROGRESS.md','scripts/generate_av1_restoration_cdef_samples.py','src/codec/av1_picture.rs','src/codec/av1_restoration.rs','src/codec/av1_restoration_filter.rs','scripts/generate_av1_superres_segmentation_samples.py','tests/av1_superres_segmentation.rs','crates/fvid-codecs/src/codec.rs','scripts/av1_tile_list_fixture.c','scripts/fixture-tools/README.md','scripts/generate_av1_tile_list_samples.py','src/codec/av1_tile_list.rs','tests/av1_tile_list.rs','tests/fixtures/playback-errors/PROVENANCE.md'}
reviewed.update(['crates/fvid-media/src/owned_aac/aac_sbr_ps.rs', 'scripts/generate_aac_ps_absence_fixtures.py', 'tests/fixtures/playback-errors/aac-ps-absence-oracles.json', 'tests/fixtures/playback-errors/aac-ps-absence-pcm.bin', 'tests/fixtures/playback-errors/he-aac-ps-absence-packets.bin', 'tests/fixtures/playback-errors/he-aac-ps-late-ps-1024-synthetic.mp4', 'tests/fixtures/playback-errors/he-aac-ps-late-ps-960-synthetic.mp4', 'tests/fixtures/playback-errors/he-aac-ps-missing-middle-1024-synthetic.mp4', 'tests/fixtures/playback-errors/he-aac-ps-missing-middle-960-synthetic.mp4', 'tests/fixtures/playback-errors/he-aac-ps-trailing-mono-1024-synthetic.mp4', 'tests/fixtures/playback-errors/he-aac-ps-trailing-mono-960-synthetic.mp4', 'tests/he_aac_ps_absence.rs'])
reviewed.update(['scripts/generate_av1_lossy_palette_samples.py', 'tests/av1_lossy_palette.rs'])
reviewed.update(['src/codec/av1_decoder.rs', 'src/codec/av1_frame.rs', 'src/codec/av1_interintra.rs', 'src/codec/av1_palette.rs', 'src/codec/av1_picture_inter.rs', 'src/codec/av1_superres.rs', 'tests/av1_mixed_inter.rs', 'tests/av1_quant_matrix_identity.rs', 'tests/av1_separate_frame.rs', 'tests/fixtures/playback-errors/av1-separate-tiles-w128-h128-groups2-incomplete.mp4', 'tests/fixtures/playback-errors/av1-separate-tiles-w128-h128-groups2.mp4', 'tests/fixtures/playback-errors/av1-separate-tiles-w128-h128-groups4-incomplete.mp4', 'tests/fixtures/playback-errors/av1-separate-tiles-w128-h128-groups4.mp4', 'tests/fixtures/playback-errors/av1-separate-tiles-w256-h128-groups4-incomplete.mp4', 'tests/fixtures/playback-errors/av1-separate-tiles-w256-h128-groups4.mp4', 'tests/fixtures/playback-errors/av1-separate-tiles-w256-h128-groups8-incomplete.mp4', 'tests/fixtures/playback-errors/av1-separate-tiles-w256-h128-groups8.mp4'])
reviewed.update(['crates/fvid-media/src/owned_mp4_video_decode.rs', 'scripts/generate_avc_field_b_longterm_samples.py', 'scripts/generate_avc_field_cabac_samples.py', 'scripts/generate_avc_field_gap_samples.py', 'scripts/generate_avc_field_reference_samples.py', 'src/codec/avc_decoder.rs', 'tests/avc_multislice.rs', 'tests/fixtures/playback-errors/README.md'])
reviewed.update(['scripts/generate_he_aac_dependent_coupling_fixtures.py', 'tests/he_aac_dependent_coupling.rs'])
reviewed.update(['docs/improvements/evidence/batch-172-before.log', 'docs/improvements/evidence/batch-172-tests-before.rs', 'docs/improvements/evidence/batch-172-tests-candidate.rs', 'scripts/generate_eac3_transient_native.py', 'src/codec/eac3_decoder.rs', 'src/codec/eac3_presentation.rs', 'src/codec/eac3_transient.rs', 'tests/eac3_presentation.rs', 'tests/eac3_raw_presentation.rs', 'tests/fixtures/playback-errors/eac3-transient-native/manifest.json', 'tests/mp4_signed_offset.rs'])
reviewed.update(['examples/hevc_colour_plane_fixture_meta.rs'])
reviewed.update(['src/codec/hevc_colour_planes.rs', 'src/codec/hevc_motion.rs', 'src/codec/hevc_picture.rs'])
reviewed.update(['src/codec/avc_reference_motion.rs'])
reviewed.update(['scripts/generate_aac_ssr_ps_fixtures.py', 'tests/aac_ssr_ps.rs', 'tests/fixtures/playback-errors/aac-ssr-ps.json'])
bad=['src/codec/cu_state_pool.rs','src/codec/hevc_ctu_context.rs','src/codec/hevc_neighbor_cache.rs','src/color/simd_rgb_convert.rs','src/container/string_interner.rs','src/codec/vp9_parallel.rs','src/codec/simd_dct.rs']
previous=g('show',head+':docs/improvements/evidence/git-integration/object-merge-resolutions.json',check=False)
report=json.loads(previous.stdout) if previous.returncode==0 else []
try:
 for b in Path('/private/tmp/fvid-merge-order.txt').read_text().splitlines():
  if b=='main' or g('merge-base','--is-ancestor',b,head,check=False).returncode==0:continue
  print('START',b,flush=True)
  r=g('merge-tree','--write-tree','--no-messages','-z',head,b,check=False)
  if r.returncode not in (0,1):raise RuntimeError(r.stderr.decode())
  tokens=r.stdout.split(b'\0');tree=tokens[0].decode();files={}
  for token in tokens[1:]:
   if not token:continue
   m,p=token.decode().split('\t',1);mode,oid,stage=m.split();files.setdefault(p,{})[stage]=(mode,oid)
  unknown=[p for p in files if p not in reviewed and p not in bad and not(p.startswith('tests/fixtures/playback-errors/av1-') and p.endswith(('.webm','.json','.obu','.yuv')))]
  Path('/private/tmp/fvid-object-pending.json').write_text(json.dumps({'branch':b,'head':head,'tree':tree,'conflicts':files,'unknown':unknown},indent=2))
  if unknown:print('NEEDS REVIEW',unknown,flush=True);break
  g('read-tree',tree,index=True)
  entries=[]
  for p,s in files.items():
   if '2' in s:mode,oid=s['2'];entries.append(f'{mode} {oid}\t{p}\n')
   else:entries.append(f'0 {"0"*40}\t{p}\n')
  for p in bad:entries.append(f'0 {"0"*40}\t{p}\n')
  g('update-index','--index-info',input=''.join(entries).encode(),index=True)
  for p in ['src/color/mod.rs','src/container/mod.rs']:
   data=g('show',':'+p,index=True).stdout.decode();out=[]
   for line in data.splitlines(keepends=True):
    if any(n in line for n in ['mod simd_rgb_convert','mod string_interner','use string_interner']):
     if out and out[-1].strip()=='#[cfg(target_arch = "x86_64")]':out.pop()
    else:out.append(line)
   new=''.join(out).encode()
   if new!=data.encode():
    oid=g('hash-object','-w','--stdin',input=new).stdout.decode().strip();g('update-index','--cacheinfo','100644',oid,p,index=True)
  report.append({'branch':b,'incoming':g('rev-parse',b).stdout.decode().strip(),'conflicts':files,'decision':'Keep reviewed superseding implementations and stricter tests; keep current paired AV1 fixtures; retain incoming versions in merge ancestry. Exclude unqualified optimization drafts.'})
  evidence=json.dumps(report,indent=2).encode()+b'\n';oid=g('hash-object','-w','--stdin',input=evidence).stdout.decode().strip()
  g('update-index','--add','--cacheinfo','100644',oid,'docs/improvements/evidence/git-integration/object-merge-resolutions.json',index=True)
  tree=g('write-tree',index=True).stdout.decode().strip()
  commit=g('commit-tree',tree,'-p',head,'-p',b,input=f'Merge preserved branch {b}\n\nRetain reviewed verified implementations and stricter regressions;\npreserve superseded drafts in ancestry without enabling unmeasured experiments.\n'.encode()).stdout.decode().strip()
  g('update-ref',ref,commit,head);head=commit
  print('MERGED',b,commit,flush=True)
 else:print('ALL MERGED',head,flush=True)
finally:
 Path('/private/tmp/fvid-object-integration-result.json').write_text(json.dumps({'ref':ref,'base':base,'head':head,'merged':len(report)},indent=2))
 Path(idx).unlink(missing_ok=True)
