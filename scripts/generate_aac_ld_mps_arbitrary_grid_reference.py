#!/usr/bin/env python3
"""Own grid-change component inputs; original C gold generation is explicit only."""
import hashlib,json,struct,subprocess,sys,tempfile
from pathlib import Path
from zipfile import ZipFile
ROOT=Path(__file__).resolve().parents[1];OUT=ROOT/'tests/fixtures/playback-errors/aac-ld-mps-arbitrary-grids';GRIDS=[23,15,12,9,7,5,4]
def sha(b):return hashlib.sha256(b).hexdigest()
def authored():
 OUT.mkdir(exist_ok=True);facts=json.loads((ROOT/'tests/fixtures/playback-errors/aac-ld-mps-prepare/facts.json').read_text());cases=[];blob=bytearray();sources={}
 for bank in [32,64,128]:
  sources[bank]=[]
  for frame in range(3):
   at=len(blob)
   for t in range(3):
    for ch in range(2):
     for q in range(bank):blob.extend(struct.pack('<2f',((frame*17+t*7+ch*3+q)%37-18)/1024,((frame*11+t*3+ch*5+q*2)%31-15)/1024))
   sources[bank].append(at)
 for bank in [32,64,128]:
  for shape in range(2):
   nodes=[[True,False,False],[False]]if shape==0 else[[True,True,False,False,False],[False]];boxes=shape+1;outputs=shape+3
   for a in GRIDS:
    for b in GRIDS:
     if a==b:continue
     blocks=[]
     for frame,bands in enumerate([a,b,a]):
      cld=[[[facts['dequantCLD'][(frame*11+ps*7+box*3+band*5)%31]for band in range(bands)]for box in range(boxes)]for ps in range(2)]
      blocks.append(dict(bands=bands,map=facts[f'QMF{bands}'][:bank],cutoffs=[bands]*boxes,endpoints=[0,2],cld=cld,source_offset=sources[bank][frame]))
     cases.append(dict(name=f'bank{bank}-shape{shape}-grid{a}-{b}-{a}',bank=bank,nodes=nodes,positions=list(range(outputs)),outputs=outputs,boxes=boxes,blocks=blocks))
 (OUT/'grid-components.json').write_text(json.dumps(dict(cases=cases,scope='252 own full-grid transitions; every ordered pair, 32/64/128 bins, simple/nested paths, return to dormant indices.'),separators=(',',':'))+'\n');(OUT/'grid-input.f32le').write_bytes(blob);return cases,bytes(blob)
def reference(archive):
 cases,source=authored();wire=bytearray(struct.pack('<I',len(cases)))
 def words(values):wire.extend(struct.pack('<'+'I'*len(values),*values))
 for c in cases:
  bands=c['blocks'][0]['bands'];words([7,c['bank'],2,c['outputs'],bands,c['boxes'],3])
  for row in c['nodes']:words([len(row)]+row)
  words(c['positions']);words([bands]*c['boxes']);words(c['blocks'][0]['map']);wire.extend(struct.pack('<2f',1.,1.))
  for f in c['blocks']:
   words([f['bands']]+f['cutoffs']+f['map']+c['positions']+[3,2]+f['endpoints'])
   for ps in f['cld']:
    for row in ps:wire.extend(struct.pack('<'+'f'*len(row),*row))
   at=f['source_offset'];wire.extend(source[at:at+3*2*c['bank']*8])
 with tempfile.TemporaryDirectory(prefix='fvid-arbitrary-grid-ref-')as tmp:
  p=Path(tmp)
  with ZipFile(archive)as z:
   for name in z.namelist():
    if name.startswith(('mp4spatialdec/src/','mp4spatialdec/include/','general/'))and name.endswith(('.c','.h')):
     assert '..'not in Path(name).parts;dest=p/name;dest.parent.mkdir(parents=True,exist_ok=True);dest.write_bytes(z.read(name))
  (p/'libtsp.h').write_text('typedef struct AFILE AFILE;\n');src=p/'mp4spatialdec/src';driver=ROOT/'scripts/support/aac_ld_mps_arbitrary_grid_reference_probe.c';exe=p/'probe';subprocess.run(['cc','-O2','-DLD_MPS','-I',str(p),'-I',str(src),'-I',str(p/'mp4spatialdec/include'),str(driver),str(src/'sac_arbitraryTree.c'),'-lm','-o',str(exe)],capture_output=True,check=True);(p/'input.bin').write_bytes(wire);subprocess.run([str(exe),str(p/'input.bin'),str(p/'output.bin')],check=True);gold=(p/'output.bin').read_bytes();original_sha=sha((src/'sac_arbitraryTree.c').read_bytes())
 (OUT/'grid-expected.f32le').write_bytes(gold);report=dict(flags=['-O2','-DLD_MPS'],scope='Original unchanged arbitraryTreeProcess; per-block grid/cutoff/map changes in caller, fixed topology, retained original indexed23-band state',programmes=len(cases),scalars=len(gold)//4,archive_sha256=sha(Path(archive).read_bytes()),original_algorithm_sha256=original_sha,harness_sha256=sha(driver.read_bytes()),input_sha256=sha(wire),expected_sha256=sha(gold),compiler=subprocess.run(['cc','--version'],capture_output=True,text=True,check=True).stdout);(ROOT/'docs/validation/aac-ld-mps-arbitrary-grids-reference-2026-11-02.json').write_text(json.dumps(report,indent=2)+'\n');print('Original C grid programmes:',len(cases),'scalars:',len(gold)//4)
if __name__=='__main__':
 if len(sys.argv)==1:authored()
 else:reference(sys.argv[1])
