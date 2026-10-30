"""Freeze numeric LD-MPS physical grids and band maps from official ISO insert."""
import hashlib,json,re,sys
from pathlib import Path
from zipfile import ZipFile
from extract_aac_ld_mps_huffman_facts import tree
ROOT=Path(__file__).resolve().parents[1]
def load(archive):
 digest=hashlib.sha256(archive.read_bytes()).hexdigest();assert digest==json.loads((ROOT/'docs/validation/aac-ld-qmf-source-2026-10-30.json').read_text())['archive_sha256']
 member='mp4spatialdec/src/sac_bitdec.c'
 with ZipFile(archive)as z:raw=z.read(member)
 text=raw.decode();facts={}
 for name in ['dequantCLD','dequantICC','dequantCPC','surroundGainTable','lfeGainTable','clipGainTable','smgTimeTable']:
  body=re.search(r'\b'+name+r'\[\]\s*=\s*\{([^}]+)\}',text).group(1);facts[name]=[float(v.rstrip('f'))if name!='smgTimeTable'else int(v)for v in body.replace(' ','').split(',')if v]
 for n in [15,12,9,7,5,4]:
  body=re.search(r'mapping_'+str(n)+r'_to_23\[[^]]+\]\s*=\s*\{([^}]+)\}',text).group(1);facts['MAP'+str(n)]=[int(v)for v in body.split(',')if v.strip()]
 facts['MAP23']=list(range(23))
 for n in [4,5,7,9,12,15,23]:
  rows=tree(text,f'kernels_{n}_to_64[MAX_HYBRID_BANDS][2]');assert len(rows)==128 and all(row[1]==1 for row in rows)
  facts['QMF'+str(n)]=[v[0]for v in rows]
 return facts,dict(archive_sha256=digest,member=member,member_sha256=hashlib.sha256(raw).hexdigest(),scope='Numeric facts only, no foreign runtime.')
def generate(archive):
 facts,source=load(archive);out=ROOT/'tests/fixtures/playback-errors/aac-ld-mps-prepare';out.mkdir(exist_ok=True)
 (out/'facts.json').write_text(json.dumps(facts,indent=2)+'\n')
 names={'dequantCLD':'CLD','dequantICC':'ICC','dequantCPC':'CPC','surroundGainTable':'SURROUND','lfeGainTable':'LFE','clipGainTable':'DOWNMIX','smgTimeTable':'SMOOTH_TIMES'}
 code='//! Numeric LD-MPS physical grids, gains, times and band maps.\n'
 for name,values in facts.items():
  integer=name.startswith('MAP')or name.startswith('QMF')or name=='smgTimeTable';typ='u16'if name=='smgTimeTable'else'u8'if integer else'f64';label=names.get(name,name)
  code+=f'pub(super) const {label}: [{typ}; {len(values)}] = ['+', '.join(str(v)for v in values)+'];\n'
 (ROOT/'crates/fvid-media/src/owned_aac/aac_ld_mps_prepare_facts.rs').write_text(code)
 (ROOT/'docs/validation/aac-ld-mps-prepare-facts-2026-10-30.json').write_text(json.dumps(source,indent=2)+'\n')
if __name__=='__main__':generate(Path(sys.argv[1]))
