#!/usr/bin/env python3
"""Explicit full-output matrix equivalence under original3D flag0/1."""
import json,struct,hashlib,subprocess,sys,tempfile,math
from pathlib import Path
from zipfile import ZipFile
ROOT=Path(__file__).resolve().parents[1];FIX=ROOT/'tests/fixtures/playback-errors'
def sha(b):return hashlib.sha256(b).hexdigest()
def main(archive):
 models=json.loads((FIX/'aac-ld-mps-assemble/manifest.json').read_text())['cases'];facts=json.loads((FIX/'aac-ld-mps-prepare/facts.json').read_text());wire=bytearray();rows=[];pairs=0
 for c in models:
  if c['tree']not in [0,1,5,6]or c['variant']!=1:continue
  params={p['id']:p for p in c['frames'][0]['parameters']};tree=c['tree'];boxes=len(c['ott_bands']);inputs=1 if tree<2 else 6;channels=5 if tree<2 else 8;outputs=6 if tree<2 else 8;width=2*channels*inputs+2*outputs*channels
  def value(key,ps,band):
   p=params[key];offset=15 if p['kind']=='CLD'else 0;return facts['dequant'+p['kind']][p['values'][ps][band-p['start']]+offset]
  for ps in range(2):
   for band in range(c['bands']):
    lfe=4 if tree==0 else 2 if tree==1 else None;active=lfe is not None and band<c['ott_bands'][lfe]
    for flag in [0,1]:
     wire.extend(struct.pack('<7I3d',flag,tree,0,0,c['inversion'],active,boxes,0.,0.,1.))
     for box in range(boxes):
      cld=value(f'ott-cld:{box}',ps,band)if band<c['ott_bands'][box]else 0.;icc=1. if box==lfe else value('ott-icc:all'if c['one_icc']else f'ott-icc:{box}',ps,band);wire.extend(struct.pack('<ddI',cld,icc,0))
    rows.append(dict(source=c['name'],set=ps,band=band,width=width));pairs+=1
 wire=struct.pack('<I',pairs*2)+wire
 with tempfile.TemporaryDirectory(prefix='fvid-unused-hrtf-')as tmp:
  work=Path(tmp)
  with ZipFile(archive)as z:
   for name in z.namelist():
    if name.startswith(('mp4spatialdec/src/','mp4spatialdec/include/','general/'))and name.endswith(('.c','.h')):
     assert '..'not in Path(name).parts;p=work/name;p.parent.mkdir(parents=True,exist_ok=True);p.write_bytes(z.read(name))
  src=work/'mp4spatialdec/src';(work/'libtsp.h').write_text('typedef struct AFILE AFILE;\n');driver=ROOT/'scripts/support/aac_ld_mps_hrtf_unused_reference_probe.c';exe=work/'probe';flags=['-O2','-DLD_MPS','-ffunction-sections','-fdata-sections','-Wl,-dead_strip'];subprocess.run(['cc']+flags+['-I',str(work),'-I',str(src),'-I',str(work/'mp4spatialdec/include'),str(driver),'-lm','-o',str(exe)],capture_output=True,check=True);(work/'input.bin').write_bytes(wire);subprocess.run([str(exe),str(work/'input.bin'),str(work/'output.bin')],check=True);output=(work/'output.bin').read_bytes();original=sha((src/'sac_calcM1andM2.c').read_bytes())
 at=0;count=0
 for row in rows:
  width=row['width']*8;a=output[at:at+width];b=output[at+width:at+2*width];assert a==b,row;assert all(math.isfinite(v[0])for v in struct.iter_unpack('<d',a)),row;at+=2*width;count+=row['width']
 assert at==len(output)
 report=dict(scope='Unchanged original full-output5151/5152/7571/7572 routines under3D flag0/1, no binaural rendering or active HRTF projection',pairs=pairs,compared_scalars=count,byte_identical=True,finite=True,trees=[0,1,5,6],archive_sha256=sha(Path(archive).read_bytes()),original_algorithm_sha256=original,source_changes=[],harness_sha256=sha(driver.read_bytes()),input_sha256=sha(wire),output_sha256=sha(output),compiler=subprocess.run(['cc','--version'],capture_output=True,text=True,check=True).stdout,flags=flags)
 (ROOT/'docs/validation/aac-ld-mps-hrtf-unused-reference-2026-11-02.json').write_text(json.dumps(report,indent=2)+'\n');print('Original full-output HRTF bypass:',pairs,'pairs,',count,'identical scalars')
if __name__=='__main__':main(sys.argv[1])
