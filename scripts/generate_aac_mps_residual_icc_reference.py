#!/usr/bin/env python3
"""Freeze original ISO mapIndexData residual-ICC results, outside ordinary tests."""
import hashlib,json,struct,subprocess,sys,tempfile,zipfile
from pathlib import Path
ROOT=Path(__file__).resolve().parents[1];OUT=ROOT/'tests/fixtures/playback-errors/aac-mps-residual-icc'
def sha(b):return hashlib.sha256(b).hexdigest()
def main(archive):
 data=Path(archive).read_bytes();assert sha(data)=='b6b27e2f9af6ee8f950af8d7074afc467e66d93372328c0ec55282fff71d1f83';m=json.loads((OUT/'parameters.json').read_text());packets=(OUT/'parameters.bin').read_bytes();records=bytearray()
 for c in m['cases']:
  for i,f in enumerate(c['frames']):
   p=packets[f['offset']:f['offset']+f['bytes']]+b'\x5a\xa5';records.extend(struct.pack('<8I',1,c['bands'],0,2,f['independent'],len(p),i==0,0));records.extend(p)
   for row in f['differences']:records.extend(struct.pack('<'+'I'*len(row),*row))
 with tempfile.TemporaryDirectory(prefix='fvid-residual-icc-')as tmp:
  work=Path(tmp)
  with zipfile.ZipFile(archive)as z:
   for name in z.namelist():
    if name.startswith(('mp4spatialdec/src/','mp4spatialdec/include/','general/'))and name.endswith(('.c','.h')):
     assert '..'not in Path(name).parts;p=work/name;p.parent.mkdir(parents=True,exist_ok=True);p.write_bytes(z.read(name))
  src=work/'mp4spatialdec/src';original=(src/'sac_bitdec.c').read_bytes();(work/'parameter-reference.c').write_bytes(original);(work/'libtsp.h').write_text('typedef struct AFILE AFILE;\n');driver=ROOT/'scripts/support/aac_mps_residual_icc_reference_probe.c';exe=work/'probe'
  args=['cc','-O2','-DLD_MPS','-ffunction-sections','-fdata-sections','-I',str(work),'-I',str(src),'-I',str(work/'mp4spatialdec/include'),str(driver),str(src/'sac_nlc_dec.c'),str(src/'sac_bitinput.c')]+[str(src/f'sac_nodes_{k}.c')for k in('cld','icc','cpc','pt0')]+(['-Wl,-dead_strip']if sys.platform=='darwin'else['-Wl,--gc-sections'])
  subprocess.run(args+['-lm','-o',str(exe)],capture_output=True,check=True,timeout=60);(work/'input.bin').write_bytes(records);result=subprocess.run([str(exe),str(work/'input.bin')],capture_output=True,text=True,check=True,timeout=30);rows=[json.loads(s)for s in result.stdout.splitlines()]
 assert len(rows)==sum(len(c['frames'])for c in m['cases']);(OUT/'reference.json').write_text(json.dumps(dict(scope='Unmodified ISO ecDataDec/mapIndexData with residual ICC differences. No interpolation or invalid dequantization index executed.',frames=rows),separators=(',',':'))+'\n')
 report=dict(archive_sha256=sha(data),original_source_sha256=sha(original),driver_sha256=sha(driver.read_bytes()),generator_sha256=sha(Path(__file__).read_bytes()),input_sha256=sha(records),stdout_sha256=sha(result.stdout.encode()),frames=len(rows),scope='Unmodified reference parameter mapping only. Shared-ICC per-box materialization and spatial matrix integration are separate owned tests. No residual PCM/playback claim.')
 (ROOT/'docs/validation/aac-mps-residual-icc-reference-2026-10-31.json').write_text(json.dumps(report,indent=2)+'\n');print(len(rows),'original ISO residual-ICC parameter frames')
if __name__=='__main__':main(sys.argv[1])
