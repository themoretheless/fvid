#!/usr/bin/env python3
"""Original gain-conversion component only; no claim for omitted 757/212 composition."""
import hashlib,json,subprocess,tempfile,sys
from pathlib import Path
from zipfile import ZipFile
from decimal import Decimal as D,localcontext
ROOT=Path(__file__).resolve().parents[1]
def sha(b):return hashlib.sha256(b).hexdigest()
def main(archive):
 with tempfile.TemporaryDirectory(prefix='fvid-adg-columns-reference-')as tmp:
  work=Path(tmp)
  with ZipFile(archive)as z:
   for name in z.namelist():
    if name.startswith(('mp4spatialdec/src/','mp4spatialdec/include/','general/'))and name.endswith(('.c','.h')):
     assert '..'not in Path(name).parts;p=work/name;p.parent.mkdir(parents=True,exist_ok=True);p.write_bytes(z.read(name))
  src=work/'mp4spatialdec/src';(work/'libtsp.h').write_text('typedef struct AFILE AFILE;\n');driver=ROOT/'scripts/support/aac_ld_mps_arbdmx_gain_probe.c';exe=work/'probe';flags=['-O2','-DLD_MPS','-ffunction-sections','-fdata-sections'];subprocess.run(['cc']+flags+['-I',str(work),'-I',str(src),'-I',str(work/'mp4spatialdec/include'),str(driver),'-Wl,-dead_strip','-lm','-o',str(exe)],capture_output=True,check=True);out=subprocess.run([str(exe)],capture_output=True,text=True,check=True).stdout;original=sha((src/'sac_calcM1andM2.c').read_bytes())
 rows=[];maximum=0.
 with localcontext()as ctx:
  ctx.prec=80
  for line in out.splitlines():
   inputs,phase,ch,db,gain=line.split();expected=float(D(10)**(D(db)/20));error=abs(float(gain)-expected);maximum=max(maximum,error);assert error<2e-6;rows.append(dict(inputs=int(inputs),phase=int(phase),channel=int(ch),db=float(db),reference=float(gain),decimal=expected))
 assert len(rows)==49
 r=dict(scope='Unmodified CalculateArbDmxMtx conversion, non-residual ADG for1/6 inputs; no foreign whole757/212 ADG matrix/playback comparison',archive_sha256=sha(Path(archive).read_bytes()),original_algorithm_file_sha256=original,source_changes=[],harness_sha256=sha(driver.read_bytes()),compiler=subprocess.run(['cc','--version'],capture_output=True,text=True,check=True).stdout,flags=flags+['-Wl,-dead_strip'],maximum_absolute_error=maximum,cases=rows)
 (ROOT/'docs/validation/aac-ld-mps-arbdmx-columns-reference-2026-11-02.json').write_text(json.dumps(r,indent=2)+'\n');print('Original gain conversion:',len(rows),'values; max error',maximum)
if __name__=='__main__':main(sys.argv[1])
