"""Archive evidence hashes and remove this completed batch's own artifacts."""
from pathlib import Path
import json,hashlib,os,re,shutil,tempfile
q=Path(__file__).resolve().parent
results=json.loads((q/'qualification-results.json').read_text());assert len(results)==5 and all(v['exit_code']==0 and v['failed']==0 for v in results.values())
for n,h in json.loads((q/'batch-source.json').read_text()).items():assert hashlib.sha256(Path(n).read_bytes()).hexdigest()==h,n
hashes=[]
for name in ['mp4-concat-unselected-plan','audio-metadata-selection-window','audio-concat-plan-dispatch']:
 for p in (q.parent/name).rglob('*.log'):
  raw=p.read_bytes();normalized=('\n'.join(line.rstrip()for line in raw.decode().splitlines()).rstrip()+'\n').encode();p.write_bytes(normalized);hashes.append(dict(path=str(p.relative_to(q.parent)),raw_sha256=hashlib.sha256(raw).hexdigest(),archived_sha256=hashlib.sha256(normalized).hexdigest()))
(q/'log-hashes.json').write_text(json.dumps(hashes,indent=2)+'\n')
for kind,v in results.items():v['archive']=kind+'.log';v['archived_sha256']=hashlib.sha256((q/v['archive']).read_bytes()).hexdigest()
(q/'qualification-results.json').write_text(json.dumps(results,indent=2)+'\n')
b=Path('docs/improvements/batch-474-validation.json');j=json.loads(b.read_text());j['qualification']=results;b.write_text(json.dumps(j,indent=2)+'\n')
removed=[];retained=[];start=(q/'base.txt').stat().st_mtime-60
patterns=[r'^fvid-concat-trim-budget-parity-(\d+)-.*$',r'^fvid-unselected-plan-public-(\d+)$',r'^fvid-metadata-selection-.*-(\d+)$',r'^fvid-audio-concat-plan-.*-(\d+)$']
root=Path(tempfile.gettempdir()).resolve()
for p in root.iterdir():
 match=next((m for pattern in patterns if(m:=re.fullmatch(pattern,p.name))),None)
 if not match or p.is_symlink() or not p.is_dir() or p.stat().st_mtime<start:continue
 pid=int(match.group(1))
 try:os.kill(pid,0)
 except ProcessLookupError:pass
 except PermissionError:retained.append(dict(path=str(p),reason='process ownership unknown'));continue
 else:retained.append(dict(path=str(p),reason='PID remains live'));continue
 assert p.parent==root;shutil.rmtree(p);removed.append(dict(path=str(p),kind='completed synthetic test output;PID absent'))
target=Path('/private/tmp/fvid-concat-unselected-plan-target-20261101');assert target.is_dir()and not target.is_symlink();shutil.rmtree(target);removed.append(dict(path=str(target),kind='owned completed qualification target'))
(q/'cleanup.json').write_text(json.dumps(dict(completed=True,removed=removed,retained=retained,regression_fixtures_and_evidence_preserved=True,foreign_worktrees_stashes_and_branches_untouched=True),indent=2)+'\n')
print('Archived log hashes;removed',len(removed),'owned/completed artifact directories')
