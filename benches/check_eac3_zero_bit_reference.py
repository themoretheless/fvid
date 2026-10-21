"""Optional strict-silence audit; ordinary tests never invoke external codecs."""
import argparse,json,math,struct,subprocess,tempfile
from pathlib import Path
p=argparse.ArgumentParser();p.add_argument('--probe',type=Path,required=True);p.add_argument('--ffmpeg',default='ffmpeg');a=p.parse_args()
root=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/eac3-zero-bit'
cases=[c for c in json.loads((root/'manifest.json').read_text())['cases']if not c['allocated']and c['pattern']=='off'];results=[]
def peak(data):
 assert len(data)%4==0
 v=struct.unpack('<'+'f'*(len(data)//4),data);assert all(map(math.isfinite,v));return max(map(abs,v),default=0)
with tempfile.TemporaryDirectory(prefix='fvid-zero-bit-silence-')as tmp:
 for c in cases:
  source=root/c['video'];out=Path(tmp)/'fvid.f32le'
  subprocess.run([str(a.probe),'--pcm',str(out),str(source)],check=True,stdout=subprocess.DEVNULL)
  ref=subprocess.run([a.ffmpeg,'-v','error','-xerror','-i',str(source),'-map','0:a:0','-f','f32le','-acodec','pcm_f32le','-'],check=True,capture_output=True).stdout
  assert len(ref)==3*1536*2*4;ref=ref[256*2*4:];got=out.read_bytes();assert len(got)==len(ref)
  x=peak(got);y=peak(ref);results.append(dict(name=c['name'],mask=c['mask'],fvid_peak=x,reference_peak=y,fvid_silent=x==0,reference_silent=y==0))
version=subprocess.run([a.ffmpeg,'-version'],check=True,capture_output=True,text=True).stdout.splitlines()[0]
print(json.dumps(dict(reference_version=version,total=len(results),fvid_silent=sum(c['fvid_silent']for c in results),reference_silent=sum(c['reference_silent']for c in results),cases=results),indent=2))
raise SystemExit(0 if all(c['fvid_silent']and c['reference_silent']for c in results)else 1)
