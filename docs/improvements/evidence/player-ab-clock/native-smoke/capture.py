from pathlib import Path
import subprocess,os,time,json,hashlib
q=Path(__file__).resolve().parent
exe=Path('/private/tmp/fvid-player-ab-clock-target-20261101/debug/fvid')
fixture=Path('tests/fixtures/playback-errors/player-jump-clock/movie.mp4').resolve()
args=[str(exe),'play',str(fixture),'--no-audio','--play-and-exit']
start=time.monotonic();meta={'arguments':args,'binary_sha256':hashlib.sha256(exe.read_bytes()).hexdigest(),'fixture_sha256':hashlib.sha256(fixture.read_bytes()).hexdigest(),'source_hashes':json.loads((q.parent/'after-source.json').read_text()),'captures':[],'scope':'Basic native-window autoplay of authored10s movie;A-B control behavior separately proven by native worker/controller tests. No physical audio or hardware-codec claim.'}
with (q/'process.log').open('wb') as output:
 p=subprocess.Popen(args,stdout=output,stderr=subprocess.STDOUT,env=os.environ.copy());meta['pid']=p.pid;print('owned native player',p.pid,flush=True)
 try:
  for index,at in enumerate([2.5,6.5]):
   while time.monotonic()-start<at and p.poll() is None:time.sleep(.03)
   assert p.poll() is None,'player ended before capture'
   windows=json.loads(subprocess.check_output(['xcrun','swift',str(q/'windows.swift'),str(p.pid)],timeout=12))
   assert windows and all(w['pid']==p.pid for w in windows),'no owned window'
   w=next((w for w in windows if w['title']=='FVid'),windows[0]);assert p.poll() is None
   image=q/f'frame-{index}.png'
   r=subprocess.run(['screencapture','-x','-l',str(w['window_id']),str(image)],stdout=subprocess.PIPE,stderr=subprocess.STDOUT,timeout=8)
   assert r.returncode==0,r.stdout.decode(errors='replace')
   meta['captures'].append({'window':w,'elapsed_seconds':time.monotonic()-start,'image':image.name,'sha256':hashlib.sha256(image.read_bytes()).hexdigest()})
   print('captured',image.name,flush=True)
  rc=p.wait(timeout=18);meta.update(exit_code=rc,elapsed_seconds=time.monotonic()-start)
  assert rc==0 and meta['elapsed_seconds']>=10,meta
 finally:
  if p.poll() is None:p.terminate();p.wait(timeout=5)
  meta['process_exited']=p.poll() is not None
  (q/'report.json').write_text(json.dumps(meta,indent=2)+'\n')
print('native playback finished',meta['elapsed_seconds'],flush=True)
