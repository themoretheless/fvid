#!/usr/bin/env python3
"""Optional original apply/decor/merge chain on saved assembly endpoints."""
import pathlib,json,hashlib,struct,subprocess,sys,tempfile,zipfile,zlib
from extract_aac_ld_mps_prepare_facts import load
ROOT=pathlib.Path(__file__).resolve().parents[1]
OUT=ROOT/'tests/fixtures/playback-errors/aac-ld-mps-qmf128'
def sha(b):return hashlib.sha256(b).hexdigest()
def main(archive):
 assert sys.byteorder=='little';facts,provenance=load(archive);assert facts==json.loads((ROOT/'tests/fixtures/playback-errors/aac-ld-mps-prepare/facts.json').read_text())
 base=ROOT/'tests/fixtures/playback-errors/aac-ld-mps-assemble';m=json.loads((base/'manifest.json').read_text());oracle=json.loads((base/'oracle.json').read_text());endpoints=(base/'expected.bin').read_bytes();records=bytearray();cases=[];output_offset=0
 for index,(c,o)in enumerate(zip(m['cases'],oracle['cases'])):
  if c['variant'] != 2:continue
  tree=c['tree'];bands=c['bands'];bins=128;boxes=len(c['cutoffs']);outputs=2 if tree==7 else 6 if tree<=2 else 8
  modes=[(c['ttt'][1]if b>=c['ttt'][2]else c['ttt'][0])for b in range(bands)]if c['ttt']else[1]*bands
  for mode in (0,1):
   records.extend(struct.pack('<5I',tree,bands,bins,mode,boxes))
   for xs in(c['cutoffs'],modes,facts[f'QMF{bands}'][:bins]):records.extend(struct.pack('<'+'I'*len(xs),*xs))
   frames=[]
   for f,of in zip(c['frames'],o['frames']):
    sets=len(of['slots']);records.extend(struct.pack('<II',sets,c['slots']));records.extend(struct.pack('<'+'I'*sets,*of['slots']))
    for plane in range(3):
     offset=of['offsets'][plane];n=of['widths'][plane]*sets*4;records.extend(endpoints[offset:offset+n])
    frames.append(dict(offset=output_offset));output_offset+=2*c['slots']*outputs*bins*2*4
   cases.append(dict(name=c['name']+f'-mode{mode}',base_case=index,mode=mode,bins=bins,outputs=outputs,frames=frames))
 records=bytearray(struct.pack('<I',len(cases)))+records
 with tempfile.TemporaryDirectory(prefix='fvid-qmf-reference-')as tmp:
  work=pathlib.Path(tmp);members={}
  with zipfile.ZipFile(archive)as z:
   for n in z.namelist():
    if n.startswith(('mp4spatialdec/src/','mp4spatialdec/include/','general/'))and n.endswith(('.c','.h')):
     assert '..'not in pathlib.Path(n).parts;b=z.read(n);dest=work/n;dest.parent.mkdir(parents=True,exist_ok=True);dest.write_bytes(b);members[n]=sha(b)
  src=work/'mp4spatialdec/src';(work/'libtsp.h').write_text('typedef struct AFILE AFILE;\n');driver=ROOT/'scripts/support/aac_ld_mps_qmf128_reference_probe.c';exe=work/'probe'
  args=['cc','-O2','-DLD_MPS','-ffunction-sections','-fdata-sections','-I',str(work),'-I',str(src),'-I',str(work/'mp4spatialdec/include'),str(driver)]+[str(src/n)for n in('sac_calcM1andM2.c','sac_process.c','sac_decor.c','sac_hybfilter.c')]+(['-Wl,-dead_strip']if sys.platform=='darwin'else['-Wl,--gc-sections'])
  result=subprocess.run(args+['-lm','-pthread','-o',str(exe)],capture_output=True,timeout=60)
  if result.returncode:raise RuntimeError(result.stderr.decode())
  (work/'input.bin').write_bytes(records);subprocess.run([str(exe),str(work/'input.bin'),str(work/'output.bin')],check=True,timeout=60);expected=(work/'output.bin').read_bytes()
 assert len(expected)==output_offset
 OUT.mkdir(parents=True,exist_ok=True);(OUT/'expected.bin.zlib').write_bytes(zlib.compress(expected,9));(OUT/'manifest.json').write_text(json.dumps(dict(scope='Controlled128-bin ordinary QMF component chain only,not full frontend layouts/PCM/movie acceptance.',expected_bytes=output_offset,cases=cases),separators=(',',':'))+'\n')
 report=dict(archive_sha256=provenance['archive_sha256'],reference_members=members,driver_sha256=sha(driver.read_bytes()),generator_sha256=sha(pathlib.Path(__file__).read_bytes()),assembly_endpoints_sha256=sha(endpoints),input_sha256=sha(records),output_sha256=sha(expected),sequences=len(cases),frames=len(cases)*3,scalar_outputs=len(expected)//4,scope='Unmodified original M1 application, decorrelation, W merge and M2 application on controlled saved endpoints; both real/complex filter modes. Padded W geometry follows the qualified matrix interface, not the original frontend channel-count selection. HRTF branch is unselected and abort-guarded. Known 212 semantic fields come from the documented assembly oracle. PCM, temporal shaping and movie acceptance remain open.',units='Normalized synthetic samples scaled by32768 for original C, outputs divided back; source samples are dyadic and exactly representable in float.')
 (ROOT/'docs/validation/aac-ld-mps-qmf128-reference-2026-10-31.json').write_text(json.dumps(report,indent=2)+'\n');print(len(cases),len(expected))
if __name__=='__main__':main(pathlib.Path(sys.argv[1]))
