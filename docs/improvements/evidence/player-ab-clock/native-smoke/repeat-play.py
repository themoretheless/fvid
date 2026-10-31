from pathlib import Path
import subprocess,time,json,hashlib
q=Path(__file__).resolve().parent;exe=Path('/private/tmp/fvid-player-ab-clock-target-20261101/debug/fvid');fixture=Path('tests/fixtures/playback-errors/player-jump-clock/movie.mp4').resolve()
args=[str(exe),'play',str(fixture),'--no-audio','--rate','1','--play-and-exit'];started=time.monotonic()
with (q/'repeat-process.log').open('wb') as log:
 p=subprocess.Popen(args,stdout=log,stderr=subprocess.STDOUT)
 try:rc=p.wait(timeout=20)
 finally:
  if p.poll() is None:p.terminate();p.wait(timeout=5)
report={'arguments':args,'pid':p.pid,'exit_code':rc,'elapsed_seconds':time.monotonic()-started,'binary_sha256':hashlib.sha256(exe.read_bytes()).hexdigest(),'process_exited':p.poll() is not None}
(q/'repeat-report.json').write_text(json.dumps(report,indent=2)+'\n');print(json.dumps(report),flush=True)
assert rc==0 and report['elapsed_seconds']>=10,report
