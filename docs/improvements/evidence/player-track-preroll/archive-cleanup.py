"""Archive owned run logs before removing this batch's temporary artifacts."""
from pathlib import Path
import hashlib,json,shutil
q=Path(__file__).resolve().parent
results=json.loads((q/'qualification-results.json').read_text())
assert len(results)==9 and all(v['exit_code']==0 for v in results.values())
repeat=json.loads((q/'repeat.json').read_text());assert len(repeat['runs'])==20 and repeat['total_case_executions']==140
assert all(r['exit_code']==0 for r in repeat['runs'])
archive=q/'raw-history';archive.mkdir(exist_ok=True)
removed=[]
for p in sorted(Path('/private/tmp').glob('fvid-player-track-preroll-*')):
    assert p.parent==Path('/private/tmp') and not p.is_symlink(),p
    if p.is_file():
        assert p.suffix=='.log',p
        raw=p.read_bytes();dest=archive/p.name.removeprefix('fvid-player-track-preroll-')
        dest.write_text('\n'.join(line.rstrip() for line in raw.decode(errors='replace').splitlines()).rstrip()+'\n')
        removed.append({'path':str(p),'bytes':len(raw),'raw_sha256':hashlib.sha256(raw).hexdigest(),'archive':str(dest.relative_to(q)),'archive_sha256':hashlib.sha256(dest.read_bytes()).hexdigest()})
        p.unlink()
    elif p.is_dir():
        assert p.name=='fvid-player-track-preroll-target-20261101',p
        shutil.rmtree(p);removed.append({'path':str(p),'kind':'owned completed-run build target'})
assert not list(Path('/private/tmp').glob('fvid-player-track-preroll-*'))
(q/'cleanup.json').write_text(json.dumps({'completed':True,'removed':removed,'regression_fixtures_and_evidence_preserved':True,'foreign_worktrees_branches_stashes_preserved':True,'earliest_tmp_before_log':'qualified baseline;all final evidence archived'},indent=2)+'\n')
