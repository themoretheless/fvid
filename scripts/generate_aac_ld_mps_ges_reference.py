#!/usr/bin/env python3
"""Freeze independent GES C reference outputs. Explicit generation, not tests."""
import json,struct,hashlib,subprocess,tempfile,zipfile,sys
from pathlib import Path
ROOT=Path(__file__).resolve().parents[1];OUT=ROOT/'tests/fixtures/playback-errors/aac-ld-mps-ges'
def sha(b):return hashlib.sha256(b).hexdigest()
def main(archive):
 assert sys.byteorder=='little';manifest=json.loads((OUT/'component.json').read_text());samples=(OUT/'component-input.f32le').read_bytes();data=bytearray(struct.pack('<I',len(manifest['cases'])));cases=[];offset=0
 for case in manifest['cases']:
  tree,bands,inputs,outputs=(case[k]for k in('tree','bands','inputs','outputs'));data.extend(struct.pack('<5I',tree,bands,inputs,outputs,len(case['blocks'])));blocks=[]
  for block in case['blocks']:
   slots=block['slots'];count=[5,5,5,7,7,4,4,2][tree];data.extend(struct.pack('<2I',slots,count))
   for row in block['targets']or[None]*count:
    data.extend(struct.pack('<I',row is not None))
    if row is not None:data.extend(struct.pack('<'+'f'*slots,*row))
   n=slots*(inputs+2*outputs)*bands*8;at=block['offset'];data.extend(samples[at:at+n]);blocks.append(dict(expected_offset=offset));offset+=slots*outputs*bands*8
  cases.append(dict(blocks=blocks))
 with tempfile.TemporaryDirectory(prefix='fvid-ges-reference-')as tmp:
  work=Path(tmp);members={}
  with zipfile.ZipFile(archive)as z:
   for name in z.namelist():
    if name.startswith(('mp4spatialdec/src/','mp4spatialdec/include/','general/'))and name.endswith(('.c','.h')):
     assert '..'not in Path(name).parts;content=z.read(name);dest=work/name;dest.parent.mkdir(parents=True,exist_ok=True);dest.write_bytes(content);members[name]=sha(content)
  src=work/'mp4spatialdec/src';(work/'libtsp.h').write_text('typedef struct AFILE AFILE;\n');driver=ROOT/'scripts/support/aac_ld_mps_ges_reference_probe.c';exe=work/'probe'
  args=['cc','-O2','-DLD_MPS','-ffunction-sections','-fdata-sections','-I',str(work),'-I',str(src),'-I',str(work/'mp4spatialdec/include'),str(driver),str(src/'sac_calcM1andM2.c')]+(['-Wl,-dead_strip']if sys.platform=='darwin'else['-Wl,--gc-sections'])
  subprocess.run(args+['-lm','-o',str(exe)],check=True,capture_output=True,timeout=60)
  (work/'input.bin').write_bytes(data);subprocess.run([str(exe),str(work/'input.bin'),str(work/'output.bin')],check=True,timeout=60);expected=(work/'output.bin').read_bytes()
 assert len(expected)==offset
 (OUT/'component-expected.f32le').write_bytes(expected);(OUT/'component-oracle.json').write_text(json.dumps(dict(scope='Unmodified ISO LD GES shaping, normalized input/output, original ISO mapper and a fully initialized23-band envelope workspace. Component reference only, not a full foreign decoder.',cases=cases),separators=(',',':'))+'\n')
 report=dict(archive_sha256=sha(Path(archive).read_bytes()),reference_members=members,driver_sha256=sha(driver.read_bytes()),generator_sha256=sha(Path(__file__).read_bytes()),input_sha256=sha(data),expected_sha256=sha(expected),programmes=len(cases),blocks=sum(len(c['blocks'])for c in cases),scope='Unmodified ISO GES algorithm; reference-only compiler invocation with the unmodified ISO channel mapper. Ordinary tests require only saved byte fixtures and FVid-owned Rust.')
 (ROOT/'docs/validation/aac-ld-mps-ges-reference-2026-10-30.json').write_text(json.dumps(report,indent=2)+'\n');print(len(expected),'independent GES bytes')
if __name__=='__main__':main(sys.argv[1])
