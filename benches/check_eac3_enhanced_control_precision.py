"""Optional diagnosis of FFmpeg 9.0.2's Q23 coefficient truncation.
Keeps the primary comparison unchanged. Only temporary conventional control
mantissas are rounded down to multiples of four at exponent 10; no FVid decoder,
checked-in input or acceptance threshold is modified.
"""
import argparse,json,struct,subprocess,sys,tempfile
from pathlib import Path
p=argparse.ArgumentParser();p.add_argument('--probe',type=Path,required=True);p.add_argument('--report',type=Path,required=True);p.add_argument('--ffmpeg',default='ffmpeg');a=p.parse_args()
repo=Path(__file__).resolve().parents[1];sys.path.insert(0,str(repo/'scripts'))
import generate_eac3_enhanced_coupling as author
root=author.OUT;cases={c['name']:c for c in json.loads((root/'manifest.json').read_text())['cases']}
failed=[r for r in json.loads(a.report.read_text())['cases']if not r['passed']]
class QuantizedWriter(author.BitWriter):
 def put(self,value,width):
  # The only 16-bit field before the mantissas is the initial syncword.
  if width==16 and self.bits:value=(value//4)*4
  return super().put(value,width)
author.BitWriter=QuantizedWriter
results=[]
with tempfile.TemporaryDirectory(prefix='fvid-control-precision-')as tmp:
 for r in failed:
  c=cases[r['name']]
  raw=author.packet(c['name'].startswith('t2-'),c['begin'],c['seed'],c['merged'],c['aht'],c['basis'],True)
  source=Path(tmp)/'control.mkv';source.write_bytes(author.movie([raw]*3));out=Path(tmp)/'pcm.f32le'
  subprocess.run([str(a.probe),'--pcm',str(out),str(source)],check=True,stdout=subprocess.DEVNULL)
  ref=subprocess.run([a.ffmpeg,'-v','error','-xerror','-i',str(root/c['control_video']),'-map','0:a:0','-f','f32le','-acodec','pcm_f32le','-'],check=True,capture_output=True).stdout[256*2*4:]
  got=out.read_bytes();assert len(got)==len(ref)
  x=struct.unpack('<'+'f'*(len(got)//4),got);y=struct.unpack('<'+'f'*(len(ref)//4),ref)
  error=max(abs(u-v)for u,v in zip(x,y))
  results.append(dict(name=c['name'],primary_error=r['max_abs_error'],temporary_q23_control_error=error,diagnostic_passed=error<1e-5))
print(json.dumps(dict(primary_failures=len(failed),diagnostic_passed=sum(c['diagnostic_passed']for c in results),cases=results),indent=2))
raise SystemExit(0 if results and all(c['diagnostic_passed']for c in results)else 1)
