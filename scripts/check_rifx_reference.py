"""Optional external reference decode of authored RIFX;not an ordinary test dependency."""
import argparse,hashlib,json,subprocess
from pathlib import Path
from generate_rifx_playback import OUT
if __name__=='__main__':
 p=argparse.ArgumentParser();p.add_argument('--ffmpeg',required=True);p.add_argument('--output',required=True);a=p.parse_args();manifest=json.loads((OUT/'manifest.json').read_text());results=[]
 for c in manifest['cases']:
  source=OUT/(c['name']+'.rifx.wav');expected=(OUT/(c['name']+'.f32le')).read_bytes();r=subprocess.run([a.ffmpeg,'-nostdin','-v','error','-i',str(source),'-map','0:a:0','-c:a','pcm_f32le','-f','f32le','-'],capture_output=True);same=r.returncode==0 and r.stdout==expected;results.append(dict(file=source.name,exit_code=r.returncode,stderr=r.stderr.decode(),source_sha256=hashlib.sha256(source.read_bytes()).hexdigest(),decoded_sha256=hashlib.sha256(r.stdout).hexdigest(),expected_sha256=hashlib.sha256(expected).hexdigest(),byte_identical=same))
 report=dict(version=subprocess.check_output([a.ffmpeg,'-version'],text=True).splitlines()[0],command='ffmpeg -nostdin -v error -i FILE -map 0:a:0 -c:a pcm_f32le -f f32le -',cases=results);Path(a.output).write_text(json.dumps(report,indent=2)+'\n');print('reference',sum(r['byte_identical']for r in results),'/',len(results));raise SystemExit(0 if all(r['byte_identical']for r in results)else 1)
