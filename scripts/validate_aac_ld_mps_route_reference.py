#!/usr/bin/env python3
"""Optional frozen original ISO W-routing comparison; not ordinary tests."""
import hashlib,json,pathlib,struct,subprocess,sys,tempfile,zipfile,math
from extract_aac_ld_mps_prepare_facts import load
ROOT=pathlib.Path(__file__).resolve().parents[1]
def sha(b):return hashlib.sha256(b).hexdigest()
def main(archive):
 assert sys.byteorder=='little';facts,provenance=load(archive);assert facts==json.loads((ROOT/'tests/fixtures/playback-errors/aac-ld-mps-prepare/facts.json').read_text())
 corpus=ROOT/'tests/fixtures/playback-errors/aac-ld-mps-route';m=json.loads((corpus/'manifest.json').read_text());expected=(corpus/'expected.bin').read_bytes();records=bytearray(struct.pack('<I',len(m['cases'])))
 for c in m['cases']:
  modes=c['modes']or[1]*c['bands'];mapping=facts[f"QMF{c['bands']}"][:c['bins']]
  records.extend(struct.pack('<4I',c['tree'],c['bands'],c['bins'],len(c['cutoffs'])))
  for xs in (c['cutoffs'],modes,mapping):records.extend(struct.pack('<'+'I'*len(xs),*xs))
 with tempfile.TemporaryDirectory(prefix='fvid-route-reference-')as tmp:
  work=pathlib.Path(tmp);members={}
  with zipfile.ZipFile(archive)as z:
   for n in z.namelist():
    if n.startswith(('mp4spatialdec/src/','mp4spatialdec/include/','general/'))and n.endswith(('.c','.h')):
     assert '..'not in pathlib.Path(n).parts;b=z.read(n);dest=work/n;dest.parent.mkdir(parents=True,exist_ok=True);dest.write_bytes(b);members[n]=sha(b)
  src=work/'mp4spatialdec/src';(work/'libtsp.h').write_text('typedef struct AFILE AFILE;\n');driver=ROOT/'scripts/support/aac_ld_mps_route_reference_probe.c';exe=work/'probe'
  args=['cc','-O2','-DLD_MPS','-ffunction-sections','-fdata-sections','-I',str(work),'-I',str(src),'-I',str(work/'mp4spatialdec/include'),str(driver),str(src/'sac_calcM1andM2.c')]+(['-Wl,-dead_strip']if sys.platform=='darwin'else['-Wl,--gc-sections'])
  result=subprocess.run(args+['-lm','-o',str(exe)],capture_output=True,timeout=60)
  if result.returncode:raise RuntimeError(result.stderr.decode())
  (work/'input.bin').write_bytes(records);subprocess.run([str(exe),str(work/'input.bin'),str(work/'output.bin')],check=True,timeout=60);actual=(work/'output.bin').read_bytes()
 assert len(actual)==len(expected);maximum=0.0;worst=None
 for i,((got,),(want,))in enumerate(zip(struct.iter_unpack('<d',actual),struct.iter_unpack('<d',expected))):
  assert math.isfinite(got);e=abs(got-want)
  if e>maximum:maximum=e;worst=i
 report=dict(archive_sha256=provenance['archive_sha256'],reference_members=members,driver_sha256=sha(driver.read_bytes()),validator_sha256=sha(pathlib.Path(__file__).read_bytes()),input_sha256=sha(records),reference_output_sha256=sha(actual),oracle_sha256=sha(expected),cases=len(m['cases']),scalar_outputs=len(expected)//8,max_abs_error=maximum,worst_scalar=worst,normalization='Owned/Decimal use exact 1/sqrt(2); original C uses 0.707107f and float samples.',scope='Original unmodified W merge and residual-index lookup versus independent Decimal routing oracle. Per-bin calls qualify routing only, not physical stream configuration, PCM or movie playback.')
 (ROOT/'docs/validation/aac-ld-mps-route-reference-2026-10-30.json').write_text(json.dumps(report,indent=2)+'\n');print(maximum,len(expected)//8);assert maximum<5e-7
if __name__=='__main__':main(pathlib.Path(sys.argv[1]))
