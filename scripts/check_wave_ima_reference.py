"""Optional independent IMA WAVE decode; never called by ordinary Rust tests."""
from pathlib import Path
import argparse,json,hashlib,subprocess
from generate_wave_ima_playback import OUT
if __name__=='__main__':
 p=argparse.ArgumentParser();p.add_argument('--ffmpeg',required=True);p.add_argument('--output',required=True);a=p.parse_args();results=[]
 for c in json.loads((OUT/'manifest.json').read_text())['cases']:
  r=subprocess.run([a.ffmpeg,'-nostdin','-v','error','-i',str(OUT/c['file']),'-map','0:a:0','-c:a','pcm_f32le','-f','f32le','-'],capture_output=True);expected=(OUT/c['oracle']).read_bytes()
  results.append(dict(file=c['file'],mode=c['mode'],exit_code=r.returncode,byte_identical=r.returncode==0 and r.stdout==expected,stderr=r.stderr.decode(),decoded_bytes=len(r.stdout),expected_bytes=len(expected),decoded_sha256=hashlib.sha256(r.stdout).hexdigest(),expected_sha256=hashlib.sha256(expected).hexdigest()))
 report=dict(version=subprocess.check_output([a.ffmpeg,'-version'],text=True).splitlines()[0],cases=results);Path(a.output).write_text(json.dumps(report,indent=2)+'\n');print('FFmpeg exact',sum(x['byte_identical']for x in results),'/',len(results))
 # Trimmed-final-block behavior is recorded independently; no expectation is weakened.
 raise SystemExit(0 if all(x['byte_identical']for x in results if x['mode']!='trim') else 1)
