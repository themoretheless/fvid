"""Explicit synthetic-fixture migration audit; never run by ordinary tests."""
import argparse,hashlib,json,subprocess
from pathlib import Path
ROOT=Path(__file__).resolve().parents[1]
OUT=ROOT/'docs/improvements/evidence'
def snapshot(path):
    result=subprocess.run([str(ROOT/'target/debug/examples/xm_fixture_snapshot'),str(path)],capture_output=True,check=True)
    return dict(source_sha256=hashlib.sha256(path.read_bytes()).hexdigest(),pcm_sha256=hashlib.sha256(result.stdout).hexdigest(),pcm_bytes=len(result.stdout),metadata=json.loads(result.stderr))
def main():
    parser=argparse.ArgumentParser();parser.add_argument('stage',choices=['before','after']);stage=parser.parse_args().stage
    if stage=='before':
        paths=sorted(p for p in (ROOT/'tests/fixtures').rglob('*.xm') if 'xm-sample-index' not in p.parts and 'xm-sample-scale' not in p.parts)
        manifests=[p for p in (ROOT/'tests/fixtures/playback-errors').rglob('manifest.json') if '.xm' in p.read_text()]
        archive=OUT/'batch-095-legacy-xm'
        for p in [*paths,*manifests]:
            dst=archive/p.relative_to(ROOT);dst.parent.mkdir(parents=True,exist_ok=True)
            if dst.exists():assert dst.read_bytes()==p.read_bytes(),f'existing archive differs:{p}'
            else:dst.write_bytes(p.read_bytes())
        before={str(p.relative_to(ROOT)):snapshot(p) for p in paths}
        (OUT/'batch-095-fixture-before.json').write_text(json.dumps(before,indent=2)+'\n')
        print(f'Archived {len(paths)} authored XM files and {len(manifests)} manifests; recorded native PCM and clock snapshots.')
    else:
        before=json.loads((OUT/'batch-095-fixture-before.json').read_text());after={name:snapshot(ROOT/name) for name in before}
        (OUT/'batch-095-fixture-after.json').write_text(json.dumps(after,indent=2)+'\n')
        for name,old in before.items():
            for key in ['pcm_sha256','pcm_bytes','metadata']:assert old[key]==after[name][key],f'{name}: native {key} changed'
        print(f'{len(before)} normalized authored XM fixtures preserve native PCM SHA256, byte counts and every presentation timestamp.')
if __name__=='__main__':main()
