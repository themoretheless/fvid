#!/usr/bin/env python3
"""Explicit independent raw-matrix reference extraction; never run by tests."""
import json,struct,subprocess,tempfile,zipfile,sys,hashlib
from pathlib import Path
ROOT=Path(__file__).resolve().parents[1];FIX=ROOT/'tests/fixtures/playback-errors';OUT=FIX/'aac-ld-mps-grid'
def sha(b):return hashlib.sha256(b).hexdigest()
def main(archive):
 OUT.mkdir(exist_ok=True)
 m=json.loads((FIX/'aac-ld-mps-assemble/manifest.json').read_text())['cases'];facts=json.loads((FIX/'aac-ld-mps-prepare/facts.json').read_text());records=bytearray();selected=[];count=0
 for c in m:
  if c['variant']!=1:continue
  f=c['frames'][0];regions={r['id']:r for r in f['parameters']};tree=c['tree'];boxes=len(c['ott_bands'])
  def value(key,ps,b):
   r=regions[key];offset={'CLD':15,'ICC':0,'CPC':20}[r['kind']];return facts['dequant'+r['kind']][r['values'][ps][b-r['start']]+offset]
  for ps in range(2):
   for b in range(c['bands']):
    lfe=4 if tree==0 else 2 if tree==1 else 0 if tree in(2,3,4)else None;active=lfe is not None and b<c['ott_bands'][lfe]
    mode=0;x=y=0.;rho=1.
    if c['ttt']:
     lo,hi,split=c['ttt'];high=b>=split;mode=hi if high else lo;kind='cpc'if mode<2 else'cld';x=value(f'ttt-{kind}:0:{int(high)}:0',ps,b);y=value(f'ttt-{kind}:0:{int(high)}:1',ps,b);rho=value(f'ttt-icc:0:{int(high)}',ps,b)if mode<2 else 1.
    records.extend(struct.pack('<6I3d',tree,mode,0,c['inversion'],active,boxes,x,y,rho));count+=1
    for box in range(boxes):
     cld=value(f'ott-cld:{box}',ps,b)if b<c['ott_bands'][box]else 0.;icc=1. if box==lfe else value('ott-icc:all'if c['one_icc']else f'ott-icc:{box}',ps,b)
     records.extend(struct.pack('<ddI',cld,icc,0))
  selected.append(c)
 records=struct.pack('<I',count)+records
 with tempfile.TemporaryDirectory(prefix='fvid-grid-reference-')as tmp:
  work=Path(tmp);members={}
  with zipfile.ZipFile(archive)as z:
   for name in z.namelist():
    if name.startswith(('mp4spatialdec/src/','mp4spatialdec/include/','general/'))and name.endswith(('.c','.h')):
     assert '..'not in Path(name).parts;data=z.read(name);dest=work/name;dest.parent.mkdir(parents=True,exist_ok=True);dest.write_bytes(data);members[name]=sha(data)
  (work/'libtsp.h').write_text('typedef struct AFILE AFILE;\n');driver=ROOT/'scripts/support/aac_ld_mps_assemble_reference_probe.c';src=work/'mp4spatialdec/src';exe=work/'probe'
  args=['cc','-O2','-DLD_MPS','-ffunction-sections','-fdata-sections','-I',str(work),'-I',str(src),'-I',str(work/'mp4spatialdec/include'),str(driver)]+(['-Wl,-dead_strip']if sys.platform=='darwin'else['-Wl,--gc-sections'])
  subprocess.run(args+['-lm','-o',str(exe)],check=True,capture_output=True,timeout=60)
  (work/'input.bin').write_bytes(records);subprocess.run([str(exe),str(work/'input.bin'),str(work/'output.bin')],check=True,timeout=60);raw=(work/'output.bin').read_bytes()
 at=0;expected=bytearray();cases=[]
 for c in selected:
  tree=c['tree'];v=5 if tree<2 else 6 if tree==2 else 2 if tree==7 else 8;inputs=1 if tree in(0,1,7)else 6 if tree in(5,6)else 3;outputs=2 if tree==7 else 6 if tree<3 else 8;widths=[v*inputs*2,outputs*v,outputs*v];offset=len(expected)
  for ps in range(2):
   for b in range(c['bands']):
    n=sum(widths);terms=struct.unpack_from('<'+'d'*n,raw,at);at+=n*8;m1=terms[:widths[0]];dry=list(terms[widths[0]:widths[0]+widths[1]]);wet=terms[widths[0]+widths[1]:]
    if tree==7:dry[1]=dry[3]=0. # No residual input: independently qualified 212 auxiliary-column rule.
    expected.extend(struct.pack('<'+'d'*n,*(list(m1[::2])+list(m1[1::2])+dry+list(wet))))
  cases.append(dict(source=c['name'],tree=tree,bands=c['bands'],widths=widths,offset=offset))
 assert at==len(raw)
 (OUT/'raw-matrices.f64le').write_bytes(expected);(OUT/'raw-matrices.json').write_text(json.dumps(dict(scope='Independent ISO raw matrices for all8 trees and7 resolutions; no temporal history or PCM in this reference.',cases=cases),separators=(',',':'))+'\n')
 report=dict(archive_sha256=sha(Path(archive).read_bytes()),reference_members=members,driver_sha256=sha(driver.read_bytes()),generator_sha256=sha(Path(__file__).read_bytes()),input_sha256=sha(records),raw_reference_sha256=sha(raw),expected_sha256=sha(expected),raw_band_endpoints=count,scope='Unmodified ISO raw matrix routines. Explicit generation only; ordinary tests read saved data, without C compiler, network, foreign codec or FFmpeg.')
 (ROOT/'docs/validation/aac-ld-mps-grid-reference-2026-10-30.json').write_text(json.dumps(report,indent=2)+'\n');print(len(cases),len(expected),'independent raw matrix bytes')
if __name__=='__main__':main(sys.argv[1])
