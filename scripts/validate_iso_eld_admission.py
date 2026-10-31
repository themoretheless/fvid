#!/usr/bin/env python3
"""Optional ISO AAC-ELD admission audit; never run by ordinary tests.

--fetch downloads only selected original compressed MP4 members using bounded
HTTP ranges and the pinned ISO checksum inventory. Otherwise fully offline.
Reports every failure and exits nonzero if any stream fails; no concealment,
parameter rewriting, waveform verdict or automatic expected-failure exemption.
Reference media must stay outside tests/fixtures/playback-errors.
"""
import argparse
from datetime import datetime, timezone
from concurrent.futures import ThreadPoolExecutor, as_completed
import hashlib
import json
from pathlib import Path
import subprocess
import time
import urllib.request
import zipfile
from fetch_iso_ld_mps_conformance import BASE, Remote

INVENTORY = 'mpeg4audio-conformance2ndEd_20230125.md5'
INVENTORY_SHA256 = '865521883bbc19db177c71c00fcb57bd3619a8ae73a05a21532c3fc22cc13e92'
COUNTS = {'eld-sbr': 26, 'eld-core': 128}


def selected(name, suite):
    name = Path(name).name
    return (name.startswith('er_eld_sbr_') if suite == 'eld-sbr' else
            name.startswith('er_eld') and '_sbr_' not in name and '_ldmps_' not in name) and name.endswith('.mp4')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--suite', choices=COUNTS, required=True)
    parser.add_argument('--inputs', type=Path, required=True)
    parser.add_argument('--probe', type=Path, required=True)
    parser.add_argument('--report', type=Path, required=True)
    parser.add_argument('--fetch', action='store_true')
    parser.add_argument('--jobs', type=int, default=1, help='independent decoder processes (1..4); not a benchmark')
    args = parser.parse_args()
    if not 1 <= args.jobs <= 4:
        raise ValueError('jobs must be between1 and4')
    inputs = args.inputs.resolve()
    if 'playback-errors' in inputs.parts:
        raise ValueError('external conformance media cannot be synthetic fixtures')
    inputs.mkdir(parents=True, exist_ok=True)
    inventory = inputs/INVENTORY
    if args.fetch:
        with urllib.request.urlopen(BASE+INVENTORY, timeout=30) as response:
            inventory.write_bytes(response.read(2*1024*1024))
    data = inventory.read_bytes()
    if hashlib.sha256(data).hexdigest() != INVENTORY_SHA256:
        raise ValueError('ISO checksum inventory changed; inspect before updating the pin')
    sums = {}
    for line in data.decode().splitlines():
        digest, path = line.split(None, 1)
        path = path.lstrip('./')
        if path.startswith('compressedMp4/') and selected(path, args.suite):
            name = Path(path).name
            if name in sums:
                raise ValueError('duplicate selected ISO member')
            sums[name] = digest
    if len(sums) != COUNTS[args.suite]:
        raise ValueError('unexpected ISO suite membership')
    if args.fetch:
        with zipfile.ZipFile(Remote('compressedMp4.zip')) as archive:
            found = set()
            for info in archive.infolist():
                name = Path(info.filename).name
                if name not in sums:
                    continue
                if name in found or info.file_size > 32*1024*1024:
                    raise ValueError('duplicate or oversized conformance member')
                found.add(name)
                path = inputs/name
                if not path.exists() or hashlib.md5(path.read_bytes()).hexdigest() != sums[name]:
                    payload = archive.read(info)
                    if hashlib.md5(payload).hexdigest() != sums[name]:
                        raise ValueError('original ISO member checksum mismatch')
                    path.write_bytes(payload)
            if found != set(sums):
                raise ValueError('selected ISO archive member missing')
    # Validate the complete input set before starting any decoder process.
    provenance = {}
    for name, md5 in sums.items():
        payload = (inputs/name).read_bytes()
        if hashlib.md5(payload).hexdigest() != md5:
            raise ValueError(f'ISO MD5 mismatch: {name}')
        provenance[name] = dict(iso_md5=md5, sha256=hashlib.sha256(payload).hexdigest())
    report = dict(scope='Presented PCM decoder admission, not waveform conformance or device playback',
                  suite=args.suite, expected_files=len(sums), concurrent_jobs=args.jobs,
                  started_at=datetime.now(timezone.utc).isoformat(), complete=False,
                  source=BASE+'compressedMp4.zip', inventory_sha256=INVENTORY_SHA256,
                  probe_sha256=hashlib.sha256(args.probe.read_bytes()).hexdigest(), cases=[])
    args.report.parent.mkdir(parents=True, exist_ok=True)
    def decode(name):
        started = time.monotonic()
        run = subprocess.run([str(args.probe.resolve()), str(inputs/name)], capture_output=True, text=True, timeout=180)
        return dict(name=name, **provenance[name], status=run.returncode, stdout=run.stdout,
                    stderr=run.stderr, wall_seconds=time.monotonic()-started)

    with ThreadPoolExecutor(max_workers=args.jobs) as pool:
        for future in as_completed([pool.submit(decode, name) for name in sorted(sums)]):
            row = future.result()
            report['cases'].append(row)
            report['cases'].sort(key=lambda c: c['name'])
            args.report.write_text(json.dumps(report, indent=2)+'\n')
            print(row['name'], 'accepted' if row['status'] == 0 else row['stderr'].strip(), flush=True)
    report['complete'] = True
    report['finished_at'] = datetime.now(timezone.utc).isoformat()
    args.report.write_text(json.dumps(report, indent=2)+'\n')
    failures = sum(c['status'] != 0 for c in report['cases'])
    print(f"decoded={len(sums)-failures} failed={failures}")
    return 1 if failures else 0


if __name__ == '__main__':
    raise SystemExit(main())
