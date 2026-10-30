"""Explicit ISO-reference research check; never invoked by ordinary Rust tests.
Usage: python3 scripts/validate_aac_ld_qmf_reference.py /path/to/official.zip
No download or foreign implementation is added to FVid. Original ISO sources
are read into a temporary directory and compiled unchanged. A separate PI
control changes the translation-unit constant, not the archived source files.
"""
import hashlib,json,math,re,struct,subprocess,sys,tempfile
from pathlib import Path
from zipfile import ZipFile
ROOT=Path(__file__).resolve().parents[1]
CORPUS=ROOT/'tests/fixtures/playback-errors/aac-ld-qmf'
def floats(data,kind):return [v[0]for v in struct.iter_unpack('<'+kind,data)]
def f32(x):return struct.unpack('<f',struct.pack('<f',x))[0]
def error(a,b):
    assert len(a)==len(b)
    return max(abs(x-y)for x,y in zip(a,b))
def model(n,pcm,window,pi):
    c=window if n==64 else [(window[2*i]+window[2*i+1])/2 for i in range(320)]
    a=[[complex(math.cos(pi/(2*n)*(k+.5)*(2*j+2*n+1)),math.sin(pi/(2*n)*(k+.5)*(2*j+2*n+1)))* (64/n)for j in range(2*n)]for k in range(n)]
    output=[]
    for t in range(len(pcm)//n):
        anchor=(t+1)*n-1;fold=[]
        for j in range(2*n):
            value=0.
            for part in range(5):
                i=j+2*n*part;x=pcm[anchor-i]if anchor>=i else 0.;value=f32(value+f32(x*c[i]))
            fold.append(value)
        for k in range(n):
            re=im=0.
            for j in range(2*n):
                re=f32(re+f32(fold[j]*a[k][j].real));im=f32(im+f32(fold[j]*a[k][j].imag))
            output.extend([re,im])
    return output

def synthesize(n,flat,window,pi):
    c=window if n==64 else [(window[i*2]+window[i*2+1])/2 for i in range(320)]
    v=[];count=len(flat)//(2*n)
    for t in range(count):
        samples=[complex(flat[(t*n+k)*2],flat[(t*n+k)*2+1])for k in range(n)]
        v.append([sum((samples[k]*complex(math.cos(pi/(2*n)*(k+.5)*(2*j-4*n+1)),math.sin(pi/(2*n)*(k+.5)*(2*j-4*n+1)))/64).real for k in range(n))for j in range(2*n)])
    return [f32(sum((v[t-2*q][k]*c[2*n*q+k]if t>=2*q else 0.)+(v[t-2*q-1][n+k]*c[2*n*q+n+k]if t>=2*q+1 else 0.)for q in range(5)))for t in range(count)for k in range(n)]

def run(archive):
    provenance=json.loads((ROOT/'docs/validation/aac-ld-qmf-source-2026-10-30.json').read_text())
    assert hashlib.sha256(archive.read_bytes()).hexdigest()==provenance['archive_sha256'],'Unexpected reference archive'
    rows=[];members={};window=json.loads((CORPUS/'window.json').read_text());manifest=json.loads((CORPUS/'manifest.json').read_text())
    with tempfile.TemporaryDirectory(prefix='fvid-ld-qmf-reference-')as tmp:
        work=Path(tmp)
        with ZipFile(archive)as z:
            for name in ['sac_polyphase.c','sac_polyphase.h','sac_sbrconst.h']:
                data=z.read('mp4spatialdec/src/'+name);(work/name).write_bytes(data);members[name]=hashlib.sha256(data).hexdigest()
        text=(work/'sac_polyphase.c').read_text()
        table=text.split('#else\nconst double sacPolyphaseFilterbankCoefficients')[1].split('};')[0]
        literals=re.findall(r'[-+]?\d+\.\d+e[-+]\d+f',table,re.I)
        assert len(literals)==640
        assert [f32(float(v[:-1]))for v in literals]==window,'Window facts differ from frozen ISO insert'
        driver=ROOT/'scripts/support/aac_ld_qmf_reference_probe.c'
        (work/'precise.c').write_text('#include "sac_sbrconst.h"\n#undef PI\n#define PI 3.141592653589793238462643383279502884\n#include "sac_polyphase.c"\n')
        for control in ['original','precise']:
            exe=work/control;source=work/('sac_polyphase.c'if control=='original'else 'precise.c')
            subprocess.run(['cc','-O2','-DLD_MPS','-I',str(work),str(driver),str(source),'-lm','-o',str(exe)],check=True,timeout=60)
            for case in manifest['cases']:
                n=case['bands'];name=case['name'];a=work/'analysis';s=work/'output'
                subprocess.run([str(exe),str(n),str(CORPUS/(name+'.input.f32le')),str(a),str(s)],check=True,timeout=10)
                reference_a=floats(a.read_bytes(),'f');reference_s=floats(s.read_bytes(),'d');pcm=floats((CORPUS/(name+'.input.f32le')).read_bytes(),'f');pi=f32(math.pi)if control=='original'else math.pi
                row=dict(name=name,control=control,analysis_f64_difference=error(reference_a,floats((CORPUS/(name+'.analysis.f64le')).read_bytes(),'d')),synthesis_f64_difference=error(reference_s,floats((CORPUS/(name+'.output.f64le')).read_bytes(),'d')),analysis_float_model_error=error(reference_a,model(n,pcm,window,pi)),synthesis_float_model_error=error(reference_s,synthesize(n,reference_a,window,pi)))
                assert row['analysis_float_model_error']<1e-12,row
                assert row['synthesis_float_model_error']<1e-12,row
                rows.append(row)
            for case in manifest['synthesis_cases']:
                n=case['bands'];name=case['name'];a=work/'analysis';s=work/'output'
                subprocess.run([str(exe),str(n),str(CORPUS/(name+'.complex.f32le')),str(a),str(s),'synthesis'],check=True,timeout=10)
                flat=floats((CORPUS/(name+'.complex.f32le')).read_bytes(),'f');reference_s=floats(s.read_bytes(),'d');pi=f32(math.pi)if control=='original'else math.pi
                row=dict(name=name,control=control,synthesis_f64_difference=error(reference_s,floats((CORPUS/(name+'.output.f64le')).read_bytes(),'d')),synthesis_float_model_error=error(reference_s,synthesize(n,flat,window,pi)))
                assert row['synthesis_float_model_error']<1e-12,row
                rows.append(row)
    report=dict(archive_sha256=hashlib.sha256(archive.read_bytes()).hexdigest(),reference_members=members,driver_sha256=hashlib.sha256(driver.read_bytes()).hexdigest(),controls='Original ISO insert; independent rounding realization; separate double-PI research control. Original reference differences are retained, not treated as double-precision failures.',results=rows)
    (ROOT/'docs/validation/aac-ld-qmf-reference-2026-10-30.json').write_text(json.dumps(report,indent=2)+'\n')
    print('24 original/PI-control cases match the independent float realization; f64 differences retained.')
if __name__=='__main__':run(Path(sys.argv[1]))
