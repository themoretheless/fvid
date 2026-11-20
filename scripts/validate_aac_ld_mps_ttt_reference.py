#!/usr/bin/env python3
"""Optional original ISO comparison, excluded from ordinary tests."""
import hashlib,json,pathlib,struct,subprocess,sys,tempfile,zipfile,math
from extract_aac_ld_mps_prepare_facts import load
ROOT=pathlib.Path(__file__).resolve().parents[1]
def sha(b):return hashlib.sha256(b).hexdigest()
def main(archive):
 assert sys.byteorder=='little'
 facts,provenance=load(archive);assert facts==json.loads((ROOT/'tests/fixtures/playback-errors/aac-ld-mps-prepare/facts.json').read_text());corpus=ROOT/'tests/fixtures/playback-errors/aac-ld-mps-ttt';m=json.loads((corpus/'manifest.json').read_text());expected=(corpus/'expected.bin').read_bytes();records=bytearray(struct.pack('<I',len(m['cases'])))
 for mode,x,y,icc,residual,offset in m['cases']:
  g=facts['dequantCPC']if mode<2 else facts['dequantCLD'];shift=20 if mode<2 else 15;records.extend(struct.pack('<IdddI',mode,g[x+shift],g[y+shift],facts['dequantICC'][icc],residual))
 records.extend(struct.pack('<I',len(m['inversions'])))
 for mode,l,r,x,y,offset in m['inversions']:
  g=facts['dequantCPC']if mode<2 else facts['dequantCLD'];shift=20 if mode<2 else 15;records.extend(struct.pack('<Idddd',mode,facts['dequantCLD'][l+15],facts['dequantCLD'][r+15],g[x+shift],g[y+shift]))
 with tempfile.TemporaryDirectory(prefix='fvid-ttt-reference-')as tmp:
  work=pathlib.Path(tmp);members={}
  with zipfile.ZipFile(archive)as z:
   for n in z.namelist():
    if n.startswith(('mp4spatialdec/src/','mp4spatialdec/include/','general/'))and n.endswith(('.c','.h')):
     assert '..'not in pathlib.Path(n).parts;b=z.read(n);dest=work/n;dest.parent.mkdir(parents=True,exist_ok=True);dest.write_bytes(b);members[n]=sha(b)
  src=work/'mp4spatialdec/src';(work/'libtsp.h').write_text('typedef struct AFILE AFILE;\n');driver=ROOT/'scripts/support/aac_ld_mps_ttt_reference_probe.c';exe=work/'probe'
  args=['cc','-O2','-DLD_MPS','-ffunction-sections','-fdata-sections','-I',str(work),'-I',str(src),'-I',str(work/'mp4spatialdec/include'),str(driver)]+(['-Wl,-dead_strip']if sys.platform=='darwin'else['-Wl,--gc-sections'])
  result=subprocess.run(args+['-lm','-o',str(exe)],capture_output=True,timeout=60)
  if result.returncode:raise RuntimeError(result.stderr.decode())
  (work/'input.bin').write_bytes(records);subprocess.run([str(exe),str(work/'input.bin'),str(work/'output.bin')],check=True,timeout=60);actual=(work/'output.bin').read_bytes()
 offset=0;results={};nonfinite=[]
 for label,cases,width in [('transform',m['cases'],9),('inversion',m['inversions'],10)]:
  maximum=0.0;worst=None
  for i,c in enumerate(cases):
   got=struct.unpack_from('<'+'d'*width,actual,offset);offset+=width*8;want=struct.unpack_from('<'+'d'*width,expected,c[-1])
   if not all(math.isfinite(v)for v in got):nonfinite.append([label,i,list(got)]);continue
   error=max(abs(a-b)for a,b in zip(got,want))
   if error>maximum:maximum=error;worst=i
  results[label]=dict(cases=len(cases),max_abs_error=maximum,worst_case=worst)
 assert offset==len(actual)
 report=dict(archive_sha256=provenance['archive_sha256'],reference_members=members,driver_sha256=sha(driver.read_bytes()),validator_sha256=sha(pathlib.Path(__file__).read_bytes()),input_sha256=sha(records),reference_output_sha256=sha(actual),oracle_sha256=sha(expected),results=results,nonfinite_reference_cases=nonfinite,scope='Unmodified original ISO TTT/inversion primitives versus Decimal oracle; no full tree or spatial playback acceptance.')
 (ROOT/'docs/validation/aac-ld-mps-ttt-reference-2026-10-30.json').write_text(json.dumps(report,indent=2)+'\n');print(results,'nonfinite',len(nonfinite));assert not nonfinite;assert all(r['max_abs_error']<3e-6 for r in results.values())
if __name__=='__main__':main(pathlib.Path(sys.argv[1]))
