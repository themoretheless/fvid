#!/usr/bin/env python3
"""Explicit diagnostic of original C access to untransmitted HRTF tail data.
Not a normative mapping oracle, playback acceptance test or fixture generator.
"""
import hashlib,json,math,subprocess,sys,tempfile
from pathlib import Path
from zipfile import ZipFile
ROOT=Path(__file__).resolve().parents[1]
def sha(data):return hashlib.sha256(data).hexdigest()
def main(archive):
 driver=ROOT/'scripts/support/aac_ld_mps_hrtf_tail_probe.c'
 with tempfile.TemporaryDirectory(prefix='fvid-hrtf-tail-') as tmp:
  work=Path(tmp)
  with ZipFile(archive) as z:
   for name in z.namelist():
    if name.startswith(('mp4spatialdec/src/','mp4spatialdec/include/','general/')) and name.endswith(('.c','.h')):
     assert '..' not in Path(name).parts
     p=work/name;p.parent.mkdir(parents=True,exist_ok=True);p.write_bytes(z.read(name))
  src=work/'mp4spatialdec/src';(work/'libtsp.h').write_text('typedef struct AFILE AFILE;\n')
  flags=['-O2','-DLD_MPS','-ffunction-sections','-fdata-sections','-Wl,-dead_strip']
  subprocess.run(['cc']+flags+['-I',str(work),'-I',str(src),'-I',str(work/'mp4spatialdec/include'),str(driver),'-lm','-o',str(work/'probe')],capture_output=True,check=True)
  raw=subprocess.run([str(work/'probe')],capture_output=True,check=True).stdout
  original=sha((src/'sac_calcM1andM2.c').read_bytes())
 rows=[line.split() for line in raw.decode().splitlines()];pairs=[]
 for a,b in zip(rows[::2],rows[1::2]):
  s,h,t=map(int,a[:3]);assert list(map(int,b[:3]))==[s,h,1] and t==0
  x=list(map(float,a[3:]));y=list(map(float,b[3:]));assert all(map(math.isfinite,x+y))
  delta=max(abs(u-v) for u,v in zip(x,y));assert (delta>0)==(s>h),(s,h,delta)
  pairs.append(dict(spatial_bands=s,hrtf_bands=h,selected_band=s-1,tail_changes_matrix=delta>0,max_absolute_difference=delta))
 assert len(pairs)==49
 report=dict(scope='Source-dataflow diagnostic only: same transmitted HRTF values, different finite untransmitted tail. No normative mapping, PCM acceptance or conformance claim.',pairs=pairs,tail_sensitive_pairs=sum(p['tail_changes_matrix'] for p in pairs),archive_sha256=sha(Path(archive).read_bytes()),original_algorithm_sha256=original,source_changes=[],harness_sha256=sha(driver.read_bytes()),output_sha256=sha(raw),compiler=subprocess.run(['cc','--version'],capture_output=True,text=True,check=True).stdout,flags=flags)
 (ROOT/'docs/validation/aac-ld-mps-hrtf-tail-reference-2026-11-02.json').write_text(json.dumps(report,indent=2)+'\n')
 print('Original HRTF tail diagnostic:',len(pairs),'pairs;',report['tail_sensitive_pairs'],'tail-sensitive')
if __name__=='__main__':main(sys.argv[1])
