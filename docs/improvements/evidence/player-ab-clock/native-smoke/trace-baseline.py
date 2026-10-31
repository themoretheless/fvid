"""Temporary native exit tracing; always restore the qualified source bytes."""
from pathlib import Path
import subprocess,os,time,json,hashlib,difflib
q=Path(__file__).resolve().parent
source=Path('src/player.rs');original=source.read_bytes()
expected=json.loads((q.parent/'after-source.json').read_text())['src/player.rs']
assert hashlib.sha256(original).hexdigest()==expected
backup=q/'baseline-qualified-source-backup.rs.txt';backup.write_bytes(original)
base=(q.parent/'base.txt').read_text().strip()
baseline=subprocess.check_output(['git','show',base+':src/player.rs'])
s=baseline.decode()
s=s.replace('    fn apply(&mut self, control: Control) {','    fn apply(&mut self, control: Control) {\n        eprintln!("AUDIT_CONTROL {:?}",std::mem::discriminant(&control));',1)
s=s.replace('impl eframe::App for Player {','''impl eframe::App for Player {
    fn on_exit(&mut self) {
        eprintln!("AUDIT_EXIT paused={} ended={} interval={:?} clock={:?} rate={} error={:?}",self.paused,self.ended,self.interval,self.presentation_clock(),self.rate_milli,self.error);
    }''',1)
s=s.replace('        self.poll_open(ctx);','''        if ctx.input(|i|i.viewport().close_requested()) { eprintln!("AUDIT_CLOSE_REQUEST"); }
        if ctx.input(|i|i.key_pressed(egui::Key::Escape)) { eprintln!("AUDIT_ESCAPE"); }
        self.poll_open(ctx);''',1)
s=s.replace('        if self.should_quit() { ctx.send_viewport_cmd(egui::ViewportCommand::Close); }','''        if self.should_quit() {
            eprintln!("AUDIT_SOURCE_DONE interval={:?} clock={:?} paused={}",self.interval,self.presentation_clock(),self.paused);
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }''',1)
s=s.replace('                // Re-check after a late presentation, keeping hard stop bounds','''                eprintln!("AUDIT_PRESENT interval={:?} clock={:?} deadline_future={:?} paused={} stepping={}",self.interval,self.presentation_clock(),self.deadline.saturating_duration_since(Instant::now()),self.paused,stepping);
                // Re-check after a late presentation, keeping hard stop bounds''',1)
patch=''.join(difflib.unified_diff(baseline.decode().splitlines(True),s.splitlines(True),fromfile='a/src/player.rs',tofile='b/src/player.rs',n=0));(q/'baseline-temporary-trace.patch').write_text(patch)
env=os.environ.copy();env.update(json.loads((q.parent/'qualification-plan.json').read_text())['env']);env['FVID_PLAYER_STATS']='1'
exe=Path(env['CARGO_TARGET_DIR'])/'debug/fvid';fixture=Path('tests/fixtures/playback-errors/player-jump-clock/movie.mp4').resolve();report={'qualified_source_sha256':expected,'temporary_trace_sha256':hashlib.sha256(s.encode()).hexdigest(),'baseline_commit':base,'baseline_source_sha256':hashlib.sha256(baseline).hexdigest(),'runs':[]}
try:
 source.write_text(s)
 with(q/'baseline-trace-build.log').open('wb')as log:
  r=subprocess.run(['cargo','build','--locked','--offline','--no-default-features','--features','player','--bin','fvid'],env=env,stdout=log,stderr=subprocess.STDOUT)
 report['build_exit_code']=r.returncode;assert r.returncode==0
 report['binary_sha256']=hashlib.sha256(exe.read_bytes()).hexdigest()
 for paused in [False,True]:
  args=[str(exe),'play',str(fixture),'--no-audio','--rate','1','--play-and-exit']+(['--start-paused']if paused else [])
  started=time.monotonic()
  with(q/('baseline-trace-paused.log'if paused else'baseline-trace-playing.log')).open('wb')as log:
   p=subprocess.Popen(args,env=env,stdout=log,stderr=subprocess.STDOUT)
   try:
    while p.poll()is None and time.monotonic()-started<14:time.sleep(.03)
    result={'arguments':args,'pid':p.pid,'elapsed_seconds':time.monotonic()-started,'exit_code_before_owned_cleanup':p.poll(),'alive_at_deadline':p.poll()is None}
   finally:
    if p.poll()is None:p.terminate();p.wait(timeout=5)
   result['process_exited_after_cleanup']=p.poll()is not None
   report['runs'].append(result);print(json.dumps(result),flush=True)
finally:
 assert source.read_text()==s,'Unexpected concurrent source edit;do not overwrite'
 source.write_bytes(original);assert hashlib.sha256(source.read_bytes()).hexdigest()==expected
 report['qualified_source_restored']=True
 (q/'baseline-trace-report.json').write_text(json.dumps(report,indent=2)+'\n')
 backup.unlink()
print('qualified source restored',flush=True)
