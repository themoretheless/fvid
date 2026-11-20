#!/usr/bin/env python3
"""Explicit original-C empty ecData/stride/entropy comparison;never run by tests."""
import json,hashlib,struct,subprocess,sys,tempfile,zipfile
from pathlib import Path
ROOT=Path(__file__).resolve().parents[1];CORPUS=ROOT/'tests/fixtures/playback-errors/aac-ld-mps-empty-groups'
sha=lambda b:hashlib.sha256(b).hexdigest()
def main(archive):
 assert sha(Path(archive).read_bytes())=='b6b27e2f9af6ee8f950af8d7074afc467e66d93372328c0ec55282fff71d1f83'
 m=json.loads((CORPUS/'manifest.json').read_text());records=bytearray()
 for u in m['units']:
  wire=bytes.fromhex(u['wire'])+b'\x5a\xa5';records.extend(struct.pack('<4I',{'CLD':0,'ICC':1,'CPC':2}[u['kind']],u['independent'],u['stride_code'],len(wire)));records.extend(wire)
 members={}
 with tempfile.TemporaryDirectory(prefix='fvid-empty-groups-reference-')as tmp:
  work=Path(tmp)
  with zipfile.ZipFile(archive)as z:
   for name in z.namelist():
    if name.startswith(('mp4spatialdec/src/','mp4spatialdec/include/','general/'))and name.endswith(('.c','.h')):
     assert '..'not in Path(name).parts;b=z.read(name);p=work/name;p.parent.mkdir(parents=True,exist_ok=True);p.write_bytes(b);members[name]=sha(b)
  (work/'libtsp.h').write_text('typedef struct AFILE AFILE;\n');src=work/'mp4spatialdec/src';driver=ROOT/'scripts/support/aac_ld_mps_empty_groups_reference_probe.c';exe=work/'probe';args=['cc','-O2','-DLD_MPS','-ffunction-sections','-fdata-sections','-I',str(work),'-I',str(src),'-I',str(work/'mp4spatialdec/include'),str(driver),str(src/'sac_nlc_dec.c'),str(src/'sac_bitinput.c')]+[str(src/f'sac_nodes_{k}.c')for k in ['cld','icc','cpc','pt0']]+(['-Wl,-dead_strip']if sys.platform=='darwin'else['-Wl,--gc-sections'])
  built=subprocess.run(args+['-lm','-o',str(exe)],capture_output=True,timeout=60)
  if built.returncode:raise RuntimeError(built.stderr.decode())
  (work/'records').write_bytes(records);out=subprocess.run([str(exe),str(work/'records')],capture_output=True,text=True,timeout=30,check=True);rows=[json.loads(line)for line in out.stdout.splitlines()];assert len(rows)==len(m['units'])
  for u,a in zip(m['units'],rows):assert a==dict(bits=u['bits'],groups=1,map=[0,0],dummy=u['dummy'],history=0),(u,a)
 expected=dict(cases=rows,scope='Unmodified C getStrideMap and ecDataDec for empty regions. Dummy indices are consumed but map to no output bands;history cell remains zero,not the region default. No whole-codec numerical oracle.');(CORPUS/'oracle.json').write_text(json.dumps(expected,separators=(',',':'))+'\n')
 report=dict(archive_sha256=sha(Path(archive).read_bytes()),reference_members=members,driver_sha256=sha(driver.read_bytes()),generator_sha256=sha(Path(__file__).read_bytes()),input_sha256=sha(records),stdout_sha256=sha(out.stdout.encode()),oracle_sha256=sha((CORPUS/'oracle.json').read_bytes()),units=len(rows),source_unmodified=True,scope=expected['scope']);(ROOT/'docs/validation/aac-ld-mps-empty-groups-reference-2026-10-31.json').write_text(json.dumps(report,indent=2)+'\n');print(len(rows),'original-C empty-region comparisons matched')
if __name__=='__main__':main(Path(sys.argv[1]))
