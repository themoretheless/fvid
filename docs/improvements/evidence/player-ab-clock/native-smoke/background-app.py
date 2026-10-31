"""Launch only the owned temporary app in the background; retain a bundle/PID lease."""
from pathlib import Path
import subprocess,os,time,json,hashlib,plistlib,shutil
q=Path(__file__).resolve().parent
plan=json.loads((q.parent/'qualification-plan.json').read_text());target=Path(plan['env']['CARGO_TARGET_DIR']);exe=target/'debug/fvid'
assert hashlib.sha256(exe.read_bytes()).hexdigest()==json.loads((q/'restored-build.json').read_text())['binary_sha256']
app=target/'native-app/FvidABAudit.app';(app/'Contents/MacOS').mkdir(parents=True,exist_ok=True)
shutil.copy2(exe,app/'Contents/MacOS/FvidABAudit')
info={'CFBundleIdentifier':'org.fvid.codex.abaudit20261101','CFBundleName':'FvidABAudit','CFBundleExecutable':'FvidABAudit','CFBundlePackageType':'APPL','CFBundleVersion':'1','NSHighResolutionCapable':True,'LSUIElement':True}
with(app/'Contents/Info.plist').open('wb')as f:plistlib.dump(info,f)
(q/'background-Info.plist').write_bytes((app/'Contents/Info.plist').read_bytes())
fixture=Path('tests/fixtures/playback-errors/player-jump-clock/movie.mp4').resolve();args=['/usr/bin/open','-g','-W','-n',str(app),'--args','play',str(fixture),'--no-audio','--rate','1','--play-and-exit'];start=time.monotonic();report={'arguments':args,'binary_sha256':hashlib.sha256(exe.read_bytes()).hexdigest(),'fixture_sha256':hashlib.sha256(fixture.read_bytes()).hexdigest(),'captures':[],'scope':'Background LaunchServices wait plus exact bundle/PID/launch-date ownership. The open wait status is not a claimed direct app-process exit code.'}
def owned():return json.loads(subprocess.check_output(['xcrun','swift',str(q/'owned-app.swift'),str(app)],timeout=10))
lease=None
with(q/'background-launch.log').open('wb')as log:
 p=subprocess.Popen(args,stdout=log,stderr=subprocess.STDOUT,env=os.environ.copy())
 try:
  while time.monotonic()-start<4 and p.poll()is None:
   found=owned()
   if found:assert len(found)==1;lease=found[0];break
   time.sleep(.1)
  assert lease,'owned app did not stay live long enough to acquire lease'
  report['lease']=lease;print('owned background app',lease['pid'],flush=True)
  for index,at in enumerate([3,7]):
   while time.monotonic()-start<at and p.poll()is None:time.sleep(.03)
   assert p.poll()is None,'native app closed early'
   assert owned()==[lease],'owned app lease changed'
   windows=json.loads(subprocess.check_output(['xcrun','swift',str(q/'windows.swift'),str(lease['pid'])],timeout=10));assert windows and all(w['pid']==lease['pid']for w in windows)
   w=windows[0];dest=q/f'background-frame-{index}.png'
   r=subprocess.run(['screencapture','-x','-l',str(w['window_id']),str(dest)],stdout=subprocess.PIPE,stderr=subprocess.STDOUT,timeout=8);assert r.returncode==0,r.stdout
   report['captures'].append({'image':dest.name,'window':w,'elapsed_seconds':time.monotonic()-start,'sha256':hashlib.sha256(dest.read_bytes()).hexdigest()})
  rc=p.wait(timeout=18);report.update(launchservices_wait_exit_code=rc,elapsed_seconds=time.monotonic()-start,owned_app_absent=not owned())
  assert rc==0 and report['elapsed_seconds']>=10 and report['owned_app_absent'],report
 finally:
  if lease and owned()==[lease]:
   subprocess.run(['/bin/kill','-TERM',str(lease['pid'])],check=True);report['owned_cleanup_signal']='TERM after verifying unchanged bundle/PID/launch-date lease'
  if p.poll()is None:p.terminate();p.wait(timeout=5)
  report['wait_process_exited']=p.poll()is not None
  report['unregister_exit_code']=subprocess.run(['/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister','-u',str(app)],stdout=log,stderr=subprocess.STDOUT).returncode
  (q/'background-report.json').write_text(json.dumps(report,indent=2)+'\n')
print('owned background playback completed',report['elapsed_seconds'],flush=True)
