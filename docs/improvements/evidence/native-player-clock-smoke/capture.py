from pathlib import Path
import subprocess,json,os,time,hashlib
prefix=Path('/private/tmp');exe=prefix/'fvid-player-native-ui-smoke.app/Contents/MacOS/fvid';fixture=Path('tests/fixtures/playback-errors/player-jump-clock/movie.mp4').resolve();args=[str(exe),'play',str(fixture),'--no-audio','--play-and-exit'];start=time.monotonic();meta={'arguments':args,'binary_sha256':hashlib.sha256(exe.read_bytes()).hexdigest(),'fixture_sha256':hashlib.sha256(fixture.read_bytes()).hexdigest(),'captures':[],'ownership':'Parent retains the Popen child handle;checks poll() before each PID-filtered window query and selected-window capture. No other app windows captured.'};result=prefix/'fvid-player-native-ui-capture.json'
with (prefix/'fvid-player-native-ui-capture.log').open('wb') as output:
 p=subprocess.Popen(args,stdout=output,stderr=subprocess.STDOUT,env=os.environ.copy());meta['pid']=p.pid;result.write_text(json.dumps(meta,indent=2)+'\n');print('capture-owned-pid',p.pid,flush=True)
 try:
  for index,at in enumerate([1.5,4.2,7.5]):
   while time.monotonic()-start<at and p.poll() is None:time.sleep(.05)
   if p.poll() is not None:break
   raw=subprocess.check_output(['xcrun','swift','/private/tmp/fvid-player-native-ui-windows.swift',str(p.pid)],timeout=8);windows=json.loads(raw);assert all(w['pid']==p.pid for w in windows)
   assert windows,'owned window not visible'
   window=next((w for w in windows if w['title']=='FVid'),windows[0]);assert p.poll() is None
   image=prefix/f'fvid-player-native-ui-frame-{index}.png';r=subprocess.run(['screencapture','-x','-l',str(window['window_id']),str(image)],stdout=subprocess.PIPE,stderr=subprocess.STDOUT,timeout=8)
   capture={'elapsed_seconds':time.monotonic()-start,'window':window,'exit_code':r.returncode,'output':r.stdout.decode(errors='replace'),'image':str(image)}
   if r.returncode==0:capture['sha256']=hashlib.sha256(image.read_bytes()).hexdigest()
   meta['captures'].append(capture);result.write_text(json.dumps(meta,indent=2)+'\n');print('capture',index,r.returncode,flush=True)
   if r.returncode!=0:break
  rc=p.wait(timeout=18);meta.update(exit_code=rc,elapsed_seconds=time.monotonic()-start);assert rc==0 and meta['elapsed_seconds']>=10,meta
 finally:
  if p.poll() is None:p.terminate();p.wait(timeout=5)
  result.write_text(json.dumps(meta,indent=2)+'\n')
print('native-playback-completed',meta['elapsed_seconds'],flush=True)
