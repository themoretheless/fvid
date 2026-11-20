"""Coupled original ISO parse/decode/EBQ/mapping comparison for qualified frames.
Research namespace/anchor/table controls are explicit; numeric code unchanged.
"""
import json,re,struct,subprocess,sys,tempfile
from pathlib import Path
from zipfile import ZipFile
from validate_aac_ld_mps_frame_reference import descriptor
from extract_aac_ld_mps_prepare_facts import load
from validate_aac_ld_mps_prepare_reference import sha
ROOT=Path(__file__).resolve().parents[1]
PREP=ROOT/'tests/fixtures/playback-errors/aac-ld-mps-prepare'
FRAMES=ROOT/'tests/fixtures/playback-errors/aac-ld-mps-frame'
def run(archive):
 assert sys.byteorder=='little';_,provenance=load(archive);oracle=json.loads((PREP/'manifest.json').read_text());m=json.loads((FRAMES/'manifest.json').read_text());configs=(FRAMES/'configs.bin').read_bytes();packets=(FRAMES/'packets.bin').read_bytes();baseline=json.loads((ROOT/'docs/validation/aac-ld-mps-frame-reference-2026-10-30.json').read_text());data=bytearray();expected=[];excluded=[]
 programs=list(zip(m['cases'],oracle['cases']))
 for o in oracle['extra_cases']:
  c=next(c for c in m['cases']if c['name']==o['base_case']);programs.append((dict(c,name=o['name']),o))
 for c,o in programs:
  assert c['name']==o['name']
  if not o.get('supported',True):excluded.extend(dict(name=c['name']+':'+str(i),reason='Unqualified energy/tree/TTT combination; no physical acceptance claimed.')for i in range(3));continue
  if any(f['opaque_residual']for f in c['frames']):excluded.extend(dict(name=c['name']+':'+str(i),reason='Opaque residual AAC; no audio acceptance claimed.')for i in range(3));continue
  cfg=bytearray(configs[c['config_offset']:c['config_offset']+c['config_bytes']]);
  if 'quantization_override'in o:cfg[2]=(cfg[2]&63)|(o['quantization_override']<<6)
  data.extend(struct.pack('<2I',len(cfg),3));data.extend(cfg);byid={r['id']:r for r in c['regions']+[r for rs in c['extension_regions']for r in rs]}
  for f,p in zip(c['frames'],o['frames']):
   rows=f['parameters']+[r for e in f['extensions']for r in e['parameters']];packet=packets[f['offset']:f['offset']+f['bytes']]+b'\x5a\xa5';data.extend(struct.pack('<2I',len(packet),len(rows)));data.extend(packet)
   for row in rows:
    d=descriptor(row,c,True);d[7]=byid[row['id']]['default'];data.extend(struct.pack('<9I',*d))
   expected.append((c,f,p))
 members={}
 with tempfile.TemporaryDirectory(prefix='fvid-physical-frame-reference-')as tmp:
  work=Path(tmp)
  with ZipFile(archive)as z:
   for name in z.namelist():
    if name.startswith(('mp4spatialdec/src/','mp4spatialdec/include/','general/'))and name.endswith(('.c','.h')):
     assert '..'not in Path(name).parts
     b=z.read(name);dest=work/name;dest.parent.mkdir(parents=True,exist_ok=True);dest.write_bytes(b);members[name]=sha(b)
  src=work/'mp4spatialdec/src';original=(src/'sac_bitdec.c').read_bytes();assert sha(original)==baseline['original_sha256'];control=original
  for patch in baseline['changes']:
   before=patch['before'].encode();after=patch['after'].encode();assert control.count(before)==1;control=control.replace(before,after)
  text=control.decode();start=text.index('decodeAndMapFrameArbTree(HANDLE_SPATIAL_DEC self)');end=text.index('void SpatialDecDecodeFrame',start);part=text[start:end]
  corrected,n=re.subn(r'(pCurBs->\s*ottCLDidxPrev,\s*)offset\+i',r'\1offset+4*self->numTttBoxes+self->numInputChannels+i',part);assert n==1
  assert corrected.count('self->quantMode,')==1
  corrected=corrected.replace('self->quantMode,','(pConfig->bsOttBandsAT[i] == 0 ? 0 : self->quantMode),')
  control=(text[:start]+corrected+text[end:]).encode()
  before=b'for(j = 0; (j < 2) && self->tttConfig[j][i].startBand < self->tttConfig[j][i].stopBand; j++ ) {'
  after=b'for(j = 0; j < 2; j++ ) { if (self->tttConfig[j][i].startBand >= self->tttConfig[j][i].stopBand) continue;'
  assert control.count(before)==1;control=control.replace(before,after)
  (work/'physical-frame-reference.c').write_bytes(control);(work/'records.bin').write_bytes(data);(work/'libtsp.h').write_text('typedef struct AFILE AFILE;\n');driver=ROOT/'scripts/support/aac_ld_mps_physical_frame_reference_probe.c';exe=work/'probe'
  args=['cc','-O2','-DLD_MPS','-ffunction-sections','-fdata-sections','-I',str(work),'-I',str(src),'-I',str(work/'mp4spatialdec/include'),str(driver),str(src/'sac_nlc_dec.c'),str(src/'sac_bitinput.c')]+[str(src/f'sac_nodes_{k}.c')for k in ['cld','icc','cpc','pt0','reshape']]
  args+=['-Wl,-dead_strip']if sys.platform=='darwin'else['-Wl,--gc-sections']
  build=subprocess.run(args+['-lm','-o',str(exe)],capture_output=True,timeout=60)
  if build.returncode:raise RuntimeError(build.stderr.decode())
  result=subprocess.run([str(exe),str(work/'records.bin')],capture_output=True,text=True,timeout=30)
  if result.returncode:raise RuntimeError((result.returncode,result.stderr,result.stdout[-1000:]))
  actual=[json.loads(s)for s in result.stdout.splitlines()];assert len(actual)==len(expected);max_error=0
  for(c,f,p),a in zip(expected,actual):
   assert a['bits']==f['bits']and a['slots']==p['slots'],(c['name'],'framing',a['slots'],p['slots'])
   assert len(a['regions'])==len(p['parameters'])
   for got,want in zip(a['regions'],p['parameters']):
    assert got['indices'][:len(f['slots'])]==want['indices'][:len(f['slots'])],(c['name'],want['id'],'EBQ/history indices',got['indices'],want['indices'])
    assert len(got['values'])==len(want['values'])
    for row,expected_row in zip(got['values'],want['values']):
     assert len(row)==len(expected_row)
     for x,y in zip(row,expected_row):max_error=max(max_error,abs(x-y))
  assert max_error<3e-7,max_error
 report=dict(empty_arbitrary_ebq_control=dict(before='self->quantMode',after='(pConfig->bsOttBandsAT[i] == 0 ? 0 : self->quantMode)',scope='Bypass the unconditional EBQ energy assertion only for an empty arbitrary region; no numeric indices exist there.'),rejected_earlier_attempt=dict(cause='Original arbitrary-tree mapping asserts on nonzero quant mode with no input energy, including empty regions.',correction='Unqualified active-arbitrary EBQ combinations are now refused by Rust preparation. The research mapper bypasses EBQ only for zero-width regions; no numeric indices exist there.'),empty_ttt_control=dict(before=before.decode(),after=after.decode(),scope='Iterate both TTT regions and skip individual empty regions instead of abandoning a nonempty high region when low is empty.'),archive_sha256=provenance['archive_sha256'],reference_members=members,original_sha256=sha(original),controlled_source_sha256=sha(control),driver_sha256=sha(driver.read_bytes()),input_sha256=sha(data),stdout_sha256=sha(result.stdout.encode()),frames=len(actual),adjusted_indices='All encoded-set coupled energy-quantized indices match exactly, including dependent history. The ISO mapper extends physical levels only; its unused held index row is not an index oracle.',max_physical_level_abs_error=max_error,parser_controls=baseline['changes'],additional_mapper_control=dict(before='Arbitrary-tree mapper reads paramIdx offset+i',after='Arbitrary-tree mapper reads paramIdx offset+4*numTttBoxes+numInputChannels+i',scope='Same separated metadata namespace already used by the frame-parser control; numeric EBQ/dequantization code unchanged.'),excluded=excluded,scope='Coupled syntax/physical-parameter preparation only. No spatial matrix construction, PCM, residual audio, movie playback or seek acceptance.')
 (ROOT/'docs/validation/aac-ld-mps-physical-frame-reference-2026-10-30.json').write_text(json.dumps(report,indent=2)+'\n');print(len(actual),'coupled physical frames, physical level max abs error',max_error)
if __name__=='__main__':run(Path(sys.argv[1]))
