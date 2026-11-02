"""Optional FFmpeg reference for20 base Wave64 signals;not a Rust test dependency."""
import argparse,json,hashlib,subprocess
from pathlib import Path
from generate_wave64_playback import OUT
if __name__=='__main__':
 p=argparse.ArgumentParser();p.add_argument('--ffmpeg',required=True);p.add_argument('--output',required=True);a=p.parse_args();results=[]
 for c in json.loads((OUT/'manifest.json').read_text())['cases']:
  if not c['external']or c['variant']!='base':continue
  f=OUT/(c['name']+'.w64');r=subprocess.run([a.ffmpeg,'-nostdin','-v','error','-i',str(f),'-map','0:a:0','-c:a','pcm_f32le','-f','f32le','-'],capture_output=True);expected=(OUT/c['oracle']).read_bytes();results.append(dict(file=f.name,exit_code=r.returncode,byte_identical=r.returncode==0 and r.stdout==expected,stderr=r.stderr.decode(),decoded_sha256=hashlib.sha256(r.stdout).hexdigest(),expected_sha256=hashlib.sha256(expected).hexdigest(),source_sha256=hashlib.sha256(f.read_bytes()).hexdigest()))
 report=dict(version=subprocess.check_output([a.ffmpeg,'-version'],text=True).splitlines()[0],scope='20 base-marker PCM/float/G711/extensible signals;other variants use independently decoded RIFF controls',cases=results);Path(a.output).write_text(json.dumps(report,indent=2)+'\n');print('FFmpeg reference',sum(r['byte_identical']for r in results),'/',len(results));raise SystemExit(0 if all(r['byte_identical']for r in results)else 1)
