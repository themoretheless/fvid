#!/usr/bin/env python3
"""Explicit128-bin ISO component check; never called by ordinary tests."""
import hashlib,json,math,pathlib,struct,subprocess,sys,tempfile,zipfile
from generate_aac_ld_qmf128_fixtures import coefficients
ROOT=pathlib.Path(__file__).resolve().parents[1];OUT=ROOT/'tests/fixtures/playback-errors/aac-ld-qmf128'
def sha(p):return hashlib.sha256(p.read_bytes()).hexdigest()
def values(p,kind):return[v[0]for v in struct.iter_unpack('<'+kind,p.read_bytes())]
def f32(x):return struct.unpack('<f',struct.pack('<f',x))[0]
def error(a,b):assert len(a)==len(b);return max(abs(x-y)for x,y in zip(a,b))
def analyze(pcm,c,pi):
 n=128;a=[[complex(math.cos(pi/(2*n)*(k+.5)*(2*j+2*n+1)),math.sin(pi/(2*n)*(k+.5)*(2*j+2*n+1)))*.5 for j in range(2*n)]for k in range(n)];result=[]
 for t in range(len(pcm)//n):
  anchor=(t+1)*n-1;fold=[]
  for j in range(2*n):
   v=0.
   for p in range(5):i=j+2*n*p;v=f32(v+f32((pcm[anchor-i]if anchor>=i else 0.)*c[i]))
   fold.append(v)
  for row in a:
   re=im=0.
   for x,z in zip(fold,row):re=f32(re+f32(x*z.real));im=f32(im+f32(x*z.imag))
   result.extend((re,im))
 return result

def synthesize(flat,c,pi):
 n=128;s=[[complex(math.cos(pi/(2*n)*(k+.5)*(2*j-4*n+1)),math.sin(pi/(2*n)*(k+.5)*(2*j-4*n+1)))/64 for k in range(n)]for j in range(2*n)];v=[]
 for t in range(len(flat)//(2*n)):
  row=[complex(flat[(t*n+k)*2],flat[(t*n+k)*2+1])for k in range(n)];v.append([sum((z*w).real for z,w in zip(row,line))for line in s])
 return[f32(sum((v[t-2*p][k]*c[2*n*p+k]if t>=2*p else 0.)+(v[t-2*p-1][n+k]*c[2*n*p+n+k]if t>=2*p+1 else 0.)for p in range(5)))for t in range(len(v))for k in range(n)]

def main(archive):
 provenance=json.loads((ROOT/'docs/validation/aac-ld-qmf-source-2026-10-30.json').read_text());assert sha(archive)==provenance['archive_sha256'];m=json.loads((OUT/'manifest.json').read_text());rows=[];members={};c=coefficients()
 with tempfile.TemporaryDirectory(prefix='fvid-ld-qmf128-reference-')as tmp:
  work=pathlib.Path(tmp)
  with zipfile.ZipFile(archive)as z:
   for name in ('sac_polyphase.c','sac_polyphase.h','sac_sbrconst.h'):
    data=z.read('mp4spatialdec/src/'+name);(work/name).write_bytes(data);members[name]=hashlib.sha256(data).hexdigest()
  (work/'precise.c').write_text('#include "sac_sbrconst.h"\n#undef PI\n#define PI 3.141592653589793238462643383279502884\n#include "sac_polyphase.c"\n')
  driver=ROOT/'scripts/support/aac_ld_qmf128_reference_probe.c'
  for control in ('original','precise'):
   exe=work/control;src=work/('sac_polyphase.c'if control=='original'else'precise.c');subprocess.run(['cc','-O2','-DLD_MPS','-I',str(work),str(driver),str(src),'-lm','-o',str(exe)],check=True,timeout=60);pi=f32(math.pi)if control=='original'else math.pi
   for case in m['cases']:
    name=case['name'];a=work/'analysis';s=work/'synthesis';subprocess.run([str(exe),'128',str(OUT/(name+'.input.f32le')),str(a),str(s)],check=True,timeout=20);actual_a=values(a,'f');actual_s=values(s,'d');pcm=values(OUT/(name+'.input.f32le'),'f');row=dict(name=name,control=control,analysis_float_model_error=error(actual_a,analyze(pcm,c,pi)),synthesis_float_model_error=error(actual_s,synthesize(actual_a,c,pi)),analysis_scalar_f64_difference=error(actual_a,values(OUT/(name+'.analysis.f64le'),'d')),synthesis_scalar_f64_difference=error(actual_s,values(OUT/(name+'.output.f64le'),'d')));assert row['analysis_float_model_error']<1e-12,row;assert row['synthesis_float_model_error']<1e-12,row;rows.append(row);(OUT/(name+'.'+control+'.analysis.f32le')).write_bytes(a.read_bytes());(OUT/(name+'.'+control+'.output.f64le')).write_bytes(s.read_bytes())
   for case in m['synthesis_cases']:
    name=case['name'];a=work/'unused';s=work/'synthesis';p=OUT/(name+'.complex.f32le');subprocess.run([str(exe),'128',str(p),str(a),str(s),'synthesis'],check=True,timeout=20);actual=values(s,'d');row=dict(name=name,control=control,synthesis_float_model_error=error(actual,synthesize(values(p,'f'),c,pi)),synthesis_scalar_f64_difference=error(actual,values(OUT/(name+'.output.f64le'),'d')));assert row['synthesis_float_model_error']<1e-12,row;rows.append(row);(OUT/(name+'.'+control+'.output.f64le')).write_bytes(s.read_bytes())
 report=dict(source=provenance['source'],archive_sha256=sha(archive),members=members,probe_sha256=sha(driver),validator_sha256=sha(pathlib.Path(__file__)),scope='Unmodified ISO LD_MPS polyphase routines at128 bands. Precise control overrides the PI macro in the translation unit only; not a whole codec/conformance oracle.',coefficient_rule='Non64 source branch averages C[floor(k*64/128)] and next cell.640 explicit LD values in a1280-sized zero-initialized array; C[640] is implicit0.',results=rows);(ROOT/'docs/validation/aac-ld-qmf128-reference-2026-10-31.json').write_text(json.dumps(report,indent=2)+'\n');print('Independent128-bin reference checks',len(rows),'maximum float-model error',max(v.get('analysis_float_model_error',0)for v in rows),max(v['synthesis_float_model_error']for v in rows))
if __name__=='__main__':main(pathlib.Path(sys.argv[1]))
