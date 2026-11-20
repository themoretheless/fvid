"""Explicit extraction of numeric LD-MPS decorrelator reflection facts only."""
import ast,hashlib,json,re,sys
from pathlib import Path
from zipfile import ZipFile
ROOT=Path(__file__).resolve().parents[1]
def load(archive):
 digest=hashlib.sha256(archive.read_bytes()).hexdigest();assert digest==json.loads((ROOT/'docs/validation/aac-ld-qmf-source-2026-10-30.json').read_text())['archive_sha256']
 member='mp4spatialdec/src/sac_decor.c'
 with ZipFile(archive)as z:raw=z.read(member)
 text=raw.decode();facts={}
 for name in ['lattice_coeff_1','lattice_coeff_2','lattice_coeff_3','lattice_deltaPhi']:
  body=re.search(r'\b'+name+r'\[[^;]+?=\s*(\{.*?\});',text,re.S).group(1)
  if '#ifndef LD_MPS'in body:body=re.sub(r'#ifndef LD_MPS.*?#else(.*?)#endif',r'\1',body,flags=re.S)
  rows=ast.literal_eval(re.sub(r'(?<=\d)f\b','',body).replace('{','[').replace('}',']'))
  order={'lattice_coeff_1':15,'lattice_coeff_2':6,'lattice_coeff_3':3,'lattice_deltaPhi':20}[name];assert all(len(r)==order for r in rows)
  rows += [[0.0]*order for _ in range(10-len(rows))];assert len(rows)==10;facts[name]=rows
 return facts,dict(archive_sha256=digest,member=member,member_sha256=hashlib.sha256(raw).hexdigest(),scope='Numeric reflection coefficients/phases only. The LD C arrays have four explicit rows and six implicit zero rows for each used reverb band; those zero initializers are preserved.')
def generate(archive):
 facts,source=load(archive);out=ROOT/'tests/fixtures/playback-errors/aac-ld-mps-upmix';out.mkdir(exist_ok=True)
 (out/'decor-facts.json').write_text(json.dumps(facts,indent=2)+'\n');code='//! Numeric LD-MPS reflection/phase facts from the frozen ISO insert.\n'
 for name,rows in facts.items():
  label={'lattice_coeff_1':'REFLECTION_15','lattice_coeff_2':'REFLECTION_6','lattice_coeff_3':'REFLECTION_3','lattice_deltaPhi':'PHASE'}[name]
  code+=f'pub(super) const {label}: [[f64; {len(rows[0])}]; 10] = '+str(rows).replace('[','[').replace(']',']')+';\n'
 (ROOT/'crates/fvid-media/src/owned_aac/aac_ld_mps_decor_facts.rs').write_text(code)
 (ROOT/'docs/validation/aac-ld-mps-decor-facts-2026-10-30.json').write_text(json.dumps(source,indent=2)+'\n')
if __name__=='__main__':generate(Path(sys.argv[1]))
