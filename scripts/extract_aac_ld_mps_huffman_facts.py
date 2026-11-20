"""Extract numeric LD-MPS codeword facts from the frozen official ISO insert.
No reference implementation code is copied. Explicit maintenance step only;
ordinary tests and generators read the saved facts without this archive.
"""
import ast,hashlib,json,re,sys
from pathlib import Path
from zipfile import ZipFile
ROOT=Path(__file__).resolve().parents[1]
def tree(text,name):
 raw=text.split(name+' =',1)[1].split(';',1)[0];return ast.literal_eval(raw.replace('{','[').replace('}',']'))
def words(nodes):
 out=[]
 def walk(index,code,width,ancestors):
  assert index not in ancestors
  for bit,child in enumerate(nodes[index]):
   next_code=code*2+bit
   if child>0:walk(child,next_code,width+1,ancestors|{index})
   else:out.append([width+1,next_code,-(child+1)])
 walk(0,0,0,set());assert max(r[0]for r in out)<=32
 return sorted(out)
def load(archive):
 known=json.loads((ROOT/'docs/validation/aac-ld-qmf-source-2026-10-30.json').read_text());digest=hashlib.sha256(archive.read_bytes()).hexdigest();assert digest==known['archive_sha256']
 tables={};members={}
 with ZipFile(archive)as z:
  p='mp4spatialdec/src/sac_nodes_pt0.c';raw=z.read(p);members[p]=hashlib.sha256(raw).hexdigest();pt=tree(raw.decode(),'huffPart0Nodes');lav=tree(raw.decode(),'huffLavIdxNodes');tables['LAV']=words(lav[0])
  for at,kind in enumerate(['CLD','ICC','CPC']):
   p=f'mp4spatialdec/src/sac_nodes_{kind.lower()}.c';raw=z.read(p);members[p]=hashlib.sha256(raw).hexdigest();data=tree(raw.decode(),'huff'+kind+'Nodes');tables[kind+'_P0']=words(pt[at])
   for diff,label in enumerate(['F','T']):
    tables[f'{kind}_{label}1']=words(data[0][diff][0])
    for i in range(4):tables[f'{kind}_{label}2_{i}']=words(data[1][diff][0][i])
 report=dict(archive_sha256=digest,members=members,scope='Numeric prefix codeword facts only. LD-MPS excludes pilot and time-pair coding; both frequency/time difference contexts and every LAV retained.',books=len(tables),codewords=sum(map(len,tables.values())))
 return tables,report
def generate(archive):
 tables,report=load(archive)
 out=ROOT/'tests/fixtures/playback-errors/aac-ld-mps-entropy';out.mkdir(exist_ok=True)
 (out/'codewords.json').write_text(json.dumps(tables,indent=2)+'\n')
 code='//! Numeric LD-MPS prefix codeword facts from ISO reference tables.\n//! Length, MSB-first codeword, symbol; -1 denotes the 2D escape leaf.\n'
 for name,rows in tables.items():code+=f'pub(super) const {name}: &[(u8,u32,i16)] = &[\n'+''.join(f'    ({length},{word},{value}),\n'for length,word,value in rows)+'];\n'
 (ROOT/'crates/fvid-media/src/owned_aac/aac_ld_mps_huffman_facts.rs').write_text(code)
 (ROOT/'docs/validation/aac-ld-mps-entropy-facts-2026-10-30.json').write_text(json.dumps(report,indent=2)+'\n');print(report['books'],report['codewords'])
if __name__=='__main__':generate(Path(sys.argv[1]))
