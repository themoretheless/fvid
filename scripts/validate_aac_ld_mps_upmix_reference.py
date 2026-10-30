"""Explicit original ISO matrix/decorrelation comparison, not ordinary tests."""
import ast,hashlib,json,re,struct,subprocess,sys,tempfile
from pathlib import Path
from zipfile import ZipFile
from extract_aac_ld_mps_decor_facts import load
from extract_aac_ld_mps_prepare_facts import load as load_physical_facts
ROOT=Path(__file__).resolve().parents[1]
CORPUS=ROOT/'tests/fixtures/playback-errors/aac-ld-mps-upmix'
def sha(b):return hashlib.sha256(b).hexdigest()
def run(archive):
 assert sys.byteorder=='little';facts,provenance=load(archive);assert facts==json.loads((CORPUS/'decor-facts.json').read_text());runtime=(ROOT/'crates/fvid-media/src/owned_aac/aac_ld_mps_decor_facts.rs').read_text()
 for key,name in [('lattice_coeff_1','REFLECTION_15'),('lattice_coeff_2','REFLECTION_6'),('lattice_coeff_3','REFLECTION_3'),('lattice_deltaPhi','PHASE')]:
  body=re.search(r'\b'+name+r':.*?=\s*(\[.*?\]);',runtime,re.S).group(1);assert ast.literal_eval(body)==facts[key]
 m=json.loads((CORPUS/'manifest.json').read_text());grids=json.loads((ROOT/'tests/fixtures/playback-errors/aac-ld-mps-prepare/facts.json').read_text());assert grids==load_physical_facts(archive)[0];blob=(CORPUS/'signals.bin').read_bytes();records=bytearray(struct.pack('<I',len(m['matrices'])))
 for row in m['matrices']:records.extend(struct.pack('<2dI',grids['dequantCLD'][row['cld']+15],grids['dequantICC'][row['icc']],row['residual']))
 records.extend(struct.pack('<I',len(m['cases'])))
 for c in m['cases']:
  records.extend(struct.pack('<5I',c['bands'],c['seed'],c['configuration'],c['mode']=='complex',c['slots']));start=c['input_offset'];records.extend(blob[start:start+c['slots']*c['bands']*16])
 members={}
 with tempfile.TemporaryDirectory(prefix='fvid-upmix-reference-')as tmp:
  work=Path(tmp)
  with ZipFile(archive)as z:
   for name in z.namelist():
    if name.startswith(('mp4spatialdec/src/','mp4spatialdec/include/','general/'))and name.endswith(('.c','.h')):
     assert '..'not in Path(name).parts
     b=z.read(name);dest=work/name;dest.parent.mkdir(parents=True,exist_ok=True);dest.write_bytes(b);members[name]=sha(b)
  src=work/'mp4spatialdec/src';(work/'libtsp.h').write_text('typedef struct AFILE AFILE;\n');(work/'input.bin').write_bytes(records);exe=work/'probe';driver=ROOT/'scripts/support/aac_ld_mps_upmix_reference_probe.c';wrapper=ROOT/'scripts/support/aac_ld_mps_ott_reference_wrapper.c'
  args=['cc','-O2','-DLD_MPS','-ffunction-sections','-fdata-sections','-I',str(work),'-I',str(src),'-I',str(work/'mp4spatialdec/include'),str(driver),str(wrapper),str(src/'sac_decor.c'),str(src/'sac_hybfilter.c')]
  args+=['-Wl,-dead_strip']if sys.platform=='darwin'else['-Wl,--gc-sections']
  build=subprocess.run(args+['-lm','-o',str(exe)],capture_output=True,timeout=60)
  if build.returncode:raise RuntimeError(build.stderr.decode())
  result=subprocess.run([str(exe),str(work/'input.bin'),str(work/'output.bin')],capture_output=True,timeout=60)
  if result.returncode:raise RuntimeError((result.returncode,result.stderr))
  actual=(work/'output.bin').read_bytes();at=0;matrix_error=0;rows=[]
  for row in m['matrices']:
   got=struct.unpack_from('<8d',actual,at);at+=64;want=[v for r in row['rows']for v in r]+row['gains'];matrix_error=max(matrix_error,max(abs(x-y)for x,y in zip(got,want)))
  for c in m['cases']:
   count=c['slots']*c['bands']*2;got=struct.unpack_from('<'+'d'*count,actual,at);at+=count*8;want=struct.unpack_from('<'+'d'*count,blob,c['output_offset']);error=max(abs(x-y)for x,y in zip(got,want));rms=(sum((x-y)**2 for x,y in zip(got,want))/count)**.5;rows.append(dict(name=c['name'],max_abs_error=error,rms_error=rms))
  assert at==len(actual)
  assert matrix_error<2e-6,matrix_error
  assert max(r['max_abs_error']for r in rows)<1e-5,sorted(rows,key=lambda r:r['max_abs_error'])[-3:]
 report=dict(archive_sha256=provenance['archive_sha256'],reference_members=members,driver_sha256=sha(driver.read_bytes()),wrapper_sha256=sha(wrapper.read_bytes()),input_sha256=sha(records),output_sha256=sha(actual),matrix_cases=len(m['matrices']),matrix_max_abs_error=matrix_error,signal_cases=len(rows),signal_complex_samples=sum(c['slots']*c['bands']for c in m['cases']),signal_max_abs_error=max(r['max_abs_error']for r in rows),normalization='Reference inputs multiplied by 32768 and outputs divided by 32768. The original ABS_THR is 1e-9*32768^2; owned DSP uses normalized full-scale-one QMF units and 1e-9.',scope='Original unmodified ISO OTT/decorrelation primitives only; no spatial movie, full tree renderer, residual decoder or PCM/playback acceptance.',results=rows)
 (ROOT/'docs/validation/aac-ld-mps-upmix-reference-2026-10-30.json').write_text(json.dumps(report,indent=2)+'\n');print(matrix_error,report['signal_max_abs_error'],report['signal_complex_samples'])
if __name__=='__main__':run(Path(sys.argv[1]))
