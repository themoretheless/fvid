#!/usr/bin/env python3
"""Authored partial-grid probes; explicit original-C zero/pattern initialization audit."""
import json,struct,hashlib,subprocess,tempfile,zipfile,sys,math
from pathlib import Path
ROOT=Path(__file__).resolve().parents[1];OUT=ROOT/'tests/fixtures/playback-errors/aac-ld-mps-arbitrary-partial'
def sha(b):return hashlib.sha256(b).hexdigest()
def authored():
 facts=json.loads((ROOT/'tests/fixtures/playback-errors/aac-ld-mps-prepare/facts.json').read_text());data=bytearray();cases=[];controls=[];errors=[]
 for bands in [12,15,23]:
  mapping=facts[f'QMF{bands}'][:32];needed=max(mapping)+1
  for cutoff in list(range(needed,bands))+[bands,needed-1]:
   for pattern in range(4):
    nodes=[[[True,False,False],[False]],[[True,True,False,False,False],[False]],[[True,False,False],[True,False,False]],[[True,True,False,False,True,False,False],[False]]][pattern];boxes=sum(sum(n)for n in nodes);outputs=2+boxes;positions=list(range(outputs));cutoffs=[cutoff]+([bands]*(boxes-1)if pattern in [1,3]else[cutoff]*(boxes-1));blocks=[]
    for frame in range(3):
     values=[((i*29+frame*13+pattern*17)%257-128)/32768 for i in range(8*2*32*2)];at=len(data);data.extend(struct.pack('<'+'f'*len(values),*values));cld=[[[facts['dequantCLD'][(frame*11+s*7+b*5+p*3)%31]for p in range(cutoffs[b])]for b in range(boxes)]for s in range(2)];blocks.append(dict(slots=8,endpoints=[3,7],cld=cld,offset=at))
    c=dict(name=f'bands{bands}-cutoff{cutoff}-shape{pattern}',tree=7,bank=32,bands=bands,cutoff=cutoff,inputs=2,outputs=outputs,nodes=nodes,cutoffs=cutoffs,positions=positions,map=mapping,fixed=[1.0,1.0],blocks=blocks)
    (controls if cutoff==bands else cases if cutoff>=needed else errors).append(c)
 OUT.mkdir(exist_ok=True);(OUT/'component-input.f32le').write_bytes(data);(OUT/'component.json').write_text(json.dumps(dict(cases=cases,controls=controls,errors=errors,scope='20 covered partial cases,12 full controls,12 active-uncovered reference-audit cases. Uncovered outputs are not playback gold.'),separators=(',',':'))+'\n');return cases,controls,errors

def reference(archive):
 positives,controls,errors=authored();cases=positives+controls+errors;raw=(OUT/'component-input.f32le').read_bytes();wire=bytearray(struct.pack('<I',len(cases)));layout=[];offset=0
 for c in cases:
  boxes=len(c['cutoffs']);wire.extend(struct.pack('<7I',7,32,2,c['outputs'],c['bands'],boxes,3))
  for row in c['nodes']:wire.extend(struct.pack('<I',len(row))+struct.pack('<'+'I'*len(row),*row))
  wire.extend(struct.pack('<'+'I'*c['outputs'],*c['positions']));wire.extend(struct.pack('<'+'I'*boxes,*c['cutoffs']));wire.extend(struct.pack('<32I',*c['map']));wire.extend(struct.pack('<2f',1.0,1.0));blocks=[]
  for b in c['blocks']:
   wire.extend(struct.pack('<'+'I'*c['outputs'],*c['positions']));wire.extend(struct.pack('<4I',8,2,3,7))
   # The source's fixed CLD workspace is initialized, but the original algorithm
   # only reads below bsOttBandsAT. No owned decoder receives padded values.
   values=[v for ps in b['cld']for row in ps for v in row+[0.0]*(c['bands']-len(row))];wire.extend(struct.pack('<'+'f'*len(values),*values));at=b['offset'];wire.extend(raw[at:at+8*2*32*8]);blocks.append(dict(offset=offset));offset+=8*c['outputs']*32*8
  layout.append(dict(name=c['name'],blocks=blocks))
 with tempfile.TemporaryDirectory(prefix='fvid-partial-reference-')as tmp:
  work=Path(tmp);members={}
  with zipfile.ZipFile(archive)as z:
   for n in z.namelist():
    if n.startswith(('mp4spatialdec/src/','mp4spatialdec/include/','general/'))and n.endswith(('.c','.h')):
     assert '..'not in Path(n).parts;data=z.read(n);p=work/n;p.parent.mkdir(parents=True,exist_ok=True);p.write_bytes(data);members[n]=sha(data)
  src=work/'mp4spatialdec/src';(work/'libtsp.h').write_text('typedef struct AFILE AFILE;\n');driver=ROOT/'scripts/support/aac_ld_mps_arbitrary_tree_reference_probe.c';(work/'input.bin').write_bytes(wire);outputs={}
  for init in ['zero','pattern']:
   exe=work/('probe-'+init);args=['cc','-O2','-DLD_MPS','-ftrivial-auto-var-init='+init,'-I',str(work),'-I',str(src),'-I',str(work/'mp4spatialdec/include'),str(driver),str(src/'sac_arbitraryTree.c'),'-lm','-o',str(exe)];subprocess.run(args,check=True,capture_output=True,timeout=60);out=work/('out-'+init);subprocess.run([str(exe),str(work/'input.bin'),str(out)],check=True,timeout=60);outputs[init]=out.read_bytes();assert len(outputs[init])==offset
 zero,pattern=outputs['zero'],outputs['pattern'];records=[];positive_end=0;below_count=0;above_differences=0
 for i,(c,record)in enumerate(zip(cases,layout)):
  different=0;nonfinite=0;low_differences=0;scalars=0
  for b in record['blocks']:
   at=b['offset'];count=8*c['outputs']*32*2
   for n in range(count):
    q=n//2%32;pos=at+n*4;a=zero[pos:pos+4];d=pattern[pos:pos+4];band=c['map'][q];av=struct.unpack('<f',a)[0];bv=struct.unpack('<f',d)[0];nonfinite+=not(math.isfinite(av)and math.isfinite(bv));different+=a!=d
    if band<c['cutoff']:low_differences+=a!=d;below_count+=1
    else:above_differences+=a!=d
    scalars+=1
   if i<len(positives)+len(controls):positive_end=max(positive_end,at+count*4)
  if i<len(positives)+len(controls):assert different==nonfinite==0,(c['name'],different,nonfinite)
  else:assert low_differences==0,(c['name'],different,low_differences)
  if i>=len(positives)+len(controls) and len(set(c['cutoffs']))>1:assert different>0,(c['name'],different)
  records.append(dict(name=c['name'],cutoffs=c['cutoffs'],covered=i<len(positives)+len(controls),different_scalars=different,nonfinite_scalars=nonfinite,below_cutoff_differences=low_differences,scalars=scalars))
 (OUT/'component-expected.f32le').write_bytes(zero[:positive_end]);(OUT/'component-oracle.json').write_text(json.dumps(dict(cases=layout[:len(positives)+len(controls)],scope='Covered partial/full only; byte-identical original-C output under both automatic-variable initialization modes.'),separators=(',',':'))+'\n')
 cache=Path('/Users/themoretheless/.cache/fvid-reference/ld-sbr');cache.mkdir(exist_ok=True)
 paths={}
 for init,data in outputs.items():p=cache/f'arbitrary-partial-{init}-output.bin';p.write_bytes(data);paths[init]=dict(path=str(p),sha256=sha(data))
 report=dict(compiler=subprocess.run(['cc','--version'],check=True,capture_output=True,text=True,timeout=20).stdout.strip(),scope='Reference qualification audit, not intended active-uncovered playback. Unmodified ISO algorithm, fully initialized caller state; only automatic-storage initialization differs.',archive_sha256=sha(Path(archive).read_bytes()),algorithm_sha256=members['mp4spatialdec/src/sac_arbitraryTree.c'],driver_sha256=sha(driver.read_bytes()),generator_sha256=sha(Path(__file__).read_bytes()),input_sha256=sha(wire),flags=['-ftrivial-auto-var-init=zero','-ftrivial-auto-var-init=pattern'],covered_cases=len(positives),full_controls=len(controls),uncovered_cases=len(errors),below_cutoff_comparisons=below_count,above_cutoff_differences=above_differences,outputs=paths,expected_sha256=sha(zero[:positive_end]),cases=records)
 (ROOT/'docs/validation/aac-ld-mps-arbitrary-partial-reference-2026-11-01.json').write_text(json.dumps(report,indent=2)+'\n');print('Qualified',len(positives),'partial and',len(controls),'full controls; uncovered audit:',[(r['name'],r['different_scalars'],r['nonfinite_scalars'])for r in records if not r['covered']])
if __name__=='__main__':reference(sys.argv[1])
