#!/usr/bin/env python3
"""Own mixed-cutoff transition stimuli; explicit original-C definedness audit."""
import json,struct,hashlib,subprocess,tempfile,sys,math
from pathlib import Path
from zipfile import ZipFile
ROOT=Path(__file__).resolve().parents[1];OUT=ROOT/'tests/fixtures/playback-errors/aac-ld-mps-arbitrary-defined'
STATES=[(b,[b,b])for b in [23,15,12,9,7,5,4]]+[(b,[c,c])for b,c in [(12,11),(15,14),(23,20),(23,21),(23,22)]]+[(b,c)for b,c in [(12,[11,12]),(12,[12,11]),(15,[14,15]),(15,[15,14]),(23,[20,23]),(23,[23,20]),(23,[21,23]),(23,[23,21]),(23,[22,23]),(23,[23,22])]]
def sha(b):return hashlib.sha256(b).hexdigest()
def authored():
 OUT.mkdir(exist_ok=True);facts=json.loads((ROOT/'tests/fixtures/playback-errors/aac-ld-mps-prepare/facts.json').read_text());source=bytearray();offsets=[];cases=[]
 for frame in range(3):
  offsets.append(len(source))
  for t in range(3):
   for ch in range(2):
    for q in range(32):source.extend(struct.pack('<2f',(1+(frame*17+t*7+ch*3+q)%37)/1024,(1+(frame*11+t*3+ch*5+q*2)%31)/1024))
 for shape,nodes in enumerate([[[True,True,False,False,False],[False]],[[True,False,True,False,False],[False]],[[True,False,False],[True,False,False]]]):
  for ai,(a,ac) in enumerate(STATES):
   for bi,(b,bc) in enumerate(STATES):
    if a==b or(ac[0]==ac[1]and bc[0]==bc[1]):continue
    blocks=[]
    for frame,(bands,cuts) in enumerate([(a,ac),(b,bc),(a,ac)]):
     cld=[[[facts['dequantCLD'][(frame*11+ps*7+box*3+band*5)%31]for band in range(cuts[box])]for box in range(2)]for ps in range(2)]
     blocks.append(dict(bands=bands,cutoffs=cuts,map=facts[f'QMF{bands}'][:32],endpoints=[0,2],cld=cld,source_offset=offsets[frame]))
    cases.append(dict(name=f'shape{shape}-state{ai}-{bi}-return',shape=shape,a=ai,b=bi,bank=32,nodes=nodes,positions=[0,1,2,3],outputs=4,boxes=2,blocks=blocks))
 (OUT/'components.json').write_text(json.dumps(dict(cases=cases,states=STATES,scope='Own mixed-cutoff transitions; reference classification is separate from authoring.'),separators=(',',':'))+'\n');(OUT/'input.f32le').write_bytes(source);return cases,bytes(source)
def reference(archive):
 cases,source=authored();wire=bytearray(struct.pack('<I',len(cases)))
 def words(values):wire.extend(struct.pack('<'+'I'*len(values),*values))
 for c in cases:
  f=c['blocks'][0];words([7,32,2,4,f['bands'],2,3])
  for row in c['nodes']:words([len(row)]+row)
  words(c['positions']+f['cutoffs']+f['map']);wire.extend(struct.pack('<2f',1.,1.))
  for f in c['blocks']:
   words([f['bands']]+f['cutoffs']+f['map']+c['positions']+[3,2]+f['endpoints'])
   for ps in f['cld']:
    for row in ps:wire.extend(struct.pack('<'+'f'*f['bands'],*(row+[0.]*(f['bands']-len(row)))))
   at=f['source_offset'];wire.extend(source[at:at+3*2*32*8])
 outputs={}
 with tempfile.TemporaryDirectory(prefix='fvid-arbitrary-defined-')as tmp:
  work=Path(tmp)
  with ZipFile(archive)as z:
   for name in z.namelist():
    if name.startswith(('mp4spatialdec/src/','mp4spatialdec/include/','general/'))and name.endswith(('.c','.h')):
     assert '..'not in Path(name).parts;p=work/name;p.parent.mkdir(parents=True,exist_ok=True);p.write_bytes(z.read(name))
  (work/'libtsp.h').write_text('typedef struct AFILE AFILE;\n');(work/'input.bin').write_bytes(wire);src=work/'mp4spatialdec/src';driver=ROOT/'scripts/support/aac_ld_mps_arbitrary_grid_reference_probe.c';original=sha((src/'sac_arbitraryTree.c').read_bytes())
  for init in ['zero','pattern']:
   exe=work/('probe-'+init);subprocess.run(['cc','-O2','-DLD_MPS','-ftrivial-auto-var-init='+init,'-I',str(work),'-I',str(src),'-I',str(work/'mp4spatialdec/include'),str(driver),str(src/'sac_arbitraryTree.c'),'-lm','-o',str(exe)],check=True,capture_output=True);p=work/('output-'+init);subprocess.run([str(exe),str(work/'input.bin'),str(p)],check=True);outputs[init]=p.read_bytes()
 gold=bytearray();rows=[];at=0;size=3*4*32*8
 for c in cases:
  first_error=None;row=dict(name=c['name'],offset=len(gold),blocks=0)
  for block in range(3):
   a=outputs['zero'][at:at+size];b=outputs['pattern'][at:at+size];at+=size
   finite=all(math.isfinite(v[0])for v in struct.iter_unpack('<f',a+b))
   if (a!=b or not finite)and first_error is None:first_error=block
   if first_error is None:gold.extend(a);row['blocks']+=1
  assert first_error!=0,c['name'];row['first_error']=first_error;rows.append(row)
 assert at==len(outputs['zero'])==len(outputs['pattern']);(OUT/'expected.f32le').write_bytes(gold);(OUT/'oracle.json').write_text(json.dumps(dict(cases=rows,scope='Only finite byte-identical prefixes under zero/pattern automatic-storage initialization are playback gold; full original-C output audit is external.'),separators=(',',':'))+'\n')
 cache=Path(archive).parent
 for init,data in outputs.items():(cache/('arbitrary-defined-'+init+'.bin')).write_bytes(data)
 report=dict(programmes=len(cases),accepted=sum(r['first_error']is None for r in rows),undefined=sum(r['first_error']is not None for r in rows),scalars=len(gold)//4,archive_sha256=sha(Path(archive).read_bytes()),original_algorithm_sha256=original,source_changes=[],harness_sha256=sha(driver.read_bytes()),input_sha256=sha(wire),gold_sha256=sha(gold),compiler=subprocess.run(['cc','--version'],capture_output=True,text=True,check=True).stdout,flags=['-O2','-DLD_MPS','-ftrivial-auto-var-init=zero/pattern'],cases=rows)
 (ROOT/'docs/validation/aac-ld-mps-arbitrary-defined-reference-2026-11-02.json').write_text(json.dumps(report,indent=2)+'\n');print('Defined source transitions:',report['accepted'],'undefined:',report['undefined'],'scalars:',report['scalars'])
if __name__=='__main__':
 if len(sys.argv)==1:authored()
 else:reference(sys.argv[1])
