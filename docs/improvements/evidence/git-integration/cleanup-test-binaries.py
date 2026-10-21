from pathlib import Path
import subprocess,json,re,shutil
r=Path('docs/improvements/evidence/git-integration');paths=[]
for n in ['fvid-final-regressions.log','fvid-final-related-regressions.log','fvid-frozen-unit-tests.log']:
 s=(Path('/private/tmp')/n).read_text(errors='replace');paths+=re.findall(r'^\s+Running .+ \(([^\n)]+)\)$',s,re.M)
active={s.strip() for s in subprocess.check_output(['ps','-Ao','comm='],text=True).splitlines()};removed=[];skipped=[];allowed=(Path.cwd()/'target/debug/deps').resolve()
for name in dict.fromkeys(paths):
 p=Path(name).resolve();assert p.parent==allowed,(p,allowed)
 if str(p) in active:skipped.append(str(p));continue
 for q in [p,Path(str(p)+'.d'),Path(str(p)+'.dSYM'),*p.parent.glob(p.name+'.*.rcgu.o')]:
  if q.is_file():size=q.stat().st_size;q.unlink();removed.append({'path':str(q),'bytes':size})
  elif q.is_dir():size=sum(v.stat().st_size for v in q.rglob('*') if v.is_file());shutil.rmtree(q);removed.append({'path':str(q),'bytes':size})
report={'removed':removed,'skipped_active':skipped,'selected_executables':len(set(paths)),'bytes':sum(x['bytes'] for x in removed)};(r/'cargo-test-artifact-cleanup.json').write_text(json.dumps(report,indent=2)+'\n');print({'selected':len(set(paths)),'removed_files':len(removed),'bytes':report['bytes'],'skipped_active':skipped})
