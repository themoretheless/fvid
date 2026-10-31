from pathlib import Path
import subprocess,time,json,os
q=Path(__file__).resolve().parent;exe=Path('/private/tmp/fvid-player-ab-clock-target-20261101/debug/fvid');fixture=Path('tests/fixtures/playback-errors/player-jump-clock/movie.mp4').resolve();args=[str(exe),'play',str(fixture),'--no-audio','--start-paused','--rate','1','--play-and-exit'];start=time.monotonic();env=os.environ.copy();env['FVID_PLAYER_STATS']='1'
with(q/'paused-process.log').open('wb')as log:
 p=subprocess.Popen(args,stdout=log,stderr=subprocess.STDOUT,env=env)
 try:
  while time.monotonic()-start<6 and p.poll() is None:time.sleep(.03)
  r={'arguments':args,'pid':p.pid,'elapsed_seconds':time.monotonic()-start,'alive_at_six_seconds':p.poll() is None,'exit_code_before_owned_cleanup':p.poll()}
 finally:
  if p.poll() is None:p.terminate();p.wait(timeout=5)
 r['process_exited_after_cleanup']=p.poll() is not None
 (q/'paused-control.json').write_text(json.dumps(r,indent=2)+'\n');print(json.dumps(r),flush=True)
