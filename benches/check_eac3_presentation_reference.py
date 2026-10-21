"""Optional expanded-presentation audit; external codecs never enter tests."""
import argparse,json,math,struct,subprocess,tempfile
from pathlib import Path
p=argparse.ArgumentParser();p.add_argument('--probe',type=Path,required=True);p.add_argument('--ffmpeg',default='ffmpeg');a=p.parse_args()
root=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/eac3-presentation'
cases=[c for c in json.loads((root/'manifest.json').read_text())['cases']if c['wave_mask']is not None];results=[]
with tempfile.TemporaryDirectory(prefix='fvid-presentation-audit-')as tmp:
 for c in cases:
  source=root/c['video'];out=Path(tmp)/'fvid.f32le'
  subprocess.run([str(a.probe),'--pcm',str(out),str(source)],check=True,stdout=subprocess.DEVNULL)
  ref=subprocess.run([a.ffmpeg,'-v','warning','-xerror','-i',str(source),'-map','0:a:0','-f','f32le','-acodec','pcm_f32le','-'],capture_output=True)
  r=dict(name=c['name'],passed=False)
  if ref.returncode:r.update(status='reference_error',returncode=ref.returncode,stderr=ref.stderr.decode(errors='replace')[:2000])
  else:
   width=c['channels'];expected=3*c['blocks']*256*width*4
   got=out.read_bytes();trimmed=ref.stdout[256*width*4:]
   if len(ref.stdout)!=expected or len(got)!=len(trimmed):r.update(status='geometry_mismatch',reference_bytes=len(ref.stdout),expected_reference_bytes=expected,fvid_bytes=len(got),stderr=ref.stderr.decode(errors='replace')[:2000])
   else:
    x=struct.unpack('<'+'f'*(len(got)//4),got);y=struct.unpack('<'+'f'*(len(trimmed)//4),trimmed)
    assert all(map(math.isfinite,x))and all(map(math.isfinite,y))
    error=max((abs(u-v)for u,v in zip(x,y)),default=0)
    r.update(status='passed'if error<1e-5 else'pcm_mismatch',passed=error<1e-5,max_abs_error=error)
  results.append(r)
version=subprocess.run([a.ffmpeg,'-version'],check=True,capture_output=True,text=True).stdout.splitlines()[0]
print(json.dumps(dict(reference_version=version,total=len(results),passed=sum(c['passed']for c in results),comparison_validated=all(c['passed']for c in results),cases=results),indent=2))
raise SystemExit(0 if all(c['passed']for c in results)else 1)
