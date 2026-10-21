"""Optional FVid-vs-FFmpeg PCM oracle check, outside production/tests/generation."""
import argparse,json,struct,subprocess,tempfile
from pathlib import Path
p=argparse.ArgumentParser();p.add_argument('--probe',type=Path,required=True);p.add_argument('--ffmpeg',default='ffmpeg');p.add_argument('--fixtures',default='eac3-aht');p.add_argument('--control',action='store_true');a=p.parse_args()
root=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors'/a.fixtures;cases=json.loads((root/'manifest.json').read_text())['cases'];results=[]
with tempfile.TemporaryDirectory(prefix='fvid-eac3-aht-reference-') as tmp:
 for c in cases:
  source=root/c['control_video' if a.control else 'video'];out=Path(tmp)/'fvid.f32le'
  subprocess.run([str(a.probe),'--pcm',str(out),str(source)],check=True,stdout=subprocess.DEVNULL)
  ref=subprocess.run([a.ffmpeg,'-v','error','-xerror','-i',str(source),'-map','0:a:0','-f','f32le','-acodec','pcm_f32le','-'],check=True,stdout=subprocess.PIPE).stdout
  # audio_probe uses the elementary decoder's documented 256-sample lead.
  channels=c.get('channels',2)
  assert len(ref)==3*c.get('blocks',6)*256*channels*4
  ref=ref[256*channels*4:]
  got=out.read_bytes();assert len(got)==len(ref),(c['name'],len(got),len(ref))
  x=struct.unpack('<'+'f'*(len(got)//4),got);y=struct.unpack('<'+'f'*(len(ref)//4),ref)
  error=max(map(lambda pair:abs(pair[0]-pair[1]),zip(x,y)),default=0)
  saturated=max((abs(u-max(-1.,min(1.,v)))for u,v in zip(x,y)),default=0)
  results.append(dict(name=c['name'],samples=len(x),max_abs_error=error,max_abs_error_saturated_reference=saturated,reference_peak=max(map(abs,y),default=0),fvid_peak=max(map(abs,x),default=0),reference_out_of_range=sum(abs(v)>1 for v in y),passed=error<1e-5,saturated_reference_passed=saturated<1e-5))
print(json.dumps(dict(control=a.control,cases=results,passed=sum(c['passed']for c in results),total=len(results)),indent=2))
raise SystemExit(0 if all(c['passed']for c in results) else 1)
