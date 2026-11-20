#!/usr/bin/env python3
"""Explicit original-C EBQ/ArbitraryTree source audit; never an ordinary test oracle."""
import hashlib,json,subprocess,sys,tempfile
from pathlib import Path
from zipfile import ZipFile
ROOT=Path(__file__).resolve().parents[1]
def sha(b):return hashlib.sha256(b).hexdigest()
def run(archive):
 with tempfile.TemporaryDirectory(prefix='fvid-arbitrary-ebq-')as tmp:
  work=Path(tmp)
  with ZipFile(archive)as z:
   for name in z.namelist():
    if name.startswith(('mp4spatialdec/src/','mp4spatialdec/include/','general/'))and name.endswith(('.c','.h')):
     assert '..'not in Path(name).parts
     p=work/name;p.parent.mkdir(parents=True,exist_ok=True);p.write_bytes(z.read(name))
  src=work/'mp4spatialdec/src';original=(src/'sac_bitdec.c').read_bytes();(work/'arbitrary-ebq-source.c').write_bytes(original);(work/'libtsp.h').write_text('typedef struct AFILE AFILE;\n');driver=ROOT/'scripts/support/aac_ld_mps_arbitrary_ebq_reference_probe.c';exe=work/'probe'
  args=['cc','-O1','-DLD_MPS','-UNDEBUG','-ffunction-sections','-fdata-sections','-I',str(work),'-I',str(src),'-I',str(work/'mp4spatialdec/include'),str(driver),'-Wl,-dead_strip','-lm','-o',str(exe)]
  subprocess.run(args,check=True,capture_output=True)
  rows=[]
  for mode in range(3):
   r=subprocess.run([str(exe),str(mode)],capture_output=True,text=True);rows.append(dict(mode=mode,exit_code=r.returncode,stdout=r.stdout,stderr=r.stderr))
  assert rows[0]['exit_code']==0
  assert all(r['exit_code']!=0 and 'ottVsTotDbIn' in r['stderr']for r in rows[1:]),rows
  result=dict(scope='Source API audit, not normative EBQ semantics or intended playback acceptance',archive_sha256=sha(Path(archive).read_bytes()),original_sac_bitdec_sha256=sha(original),source_changes=[],harness_sha256=sha(driver.read_bytes()),compiler=subprocess.run(['cc','--version'],text=True,capture_output=True,check=True).stdout,flags=['-O1','-DLD_MPS','-UNDEBUG','-ffunction-sections','-fdata-sections','-Wl,-dead_strip'],cases=rows,conclusion='The unchanged original arbitrary-tree caller passes NULL energy arrays to nonzero-quant CLD mapping; assertions fail before output. Quant-zero control returns. No normative replacement energy propagation is inferred.')
  out=ROOT/'docs/validation/aac-ld-mps-arbitrary-ebq-reference-2026-11-02.json';out.write_text(json.dumps(result,indent=2)+'\n');print(json.dumps(rows,indent=2))
if __name__=='__main__':run(sys.argv[1])
