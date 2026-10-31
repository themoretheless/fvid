#!/usr/bin/env python3
"""Paired manual CPU measurement with exact decoder assertions in each run."""
import hashlib
import json
import statistics
import subprocess
import sys
from pathlib import Path


def main():
    baseline, after, workload = map(Path, sys.argv[1:4])
    queries = int(sys.argv[4]) if len(sys.argv) > 4 else 1000
    rounds = int(sys.argv[5]) if len(sys.argv) > 5 else 7
    assert queries > 0 and rounds >= 7
    binaries = {'baseline': baseline.resolve(), 'after': after.resolve()}

    def sample(tag):
        result = subprocess.run([str(binaries[tag]), str(workload.resolve()), str(queries)], capture_output=True, text=True, timeout=600, check=True)
        row = json.loads(result.stdout)
        assert row['queries'] == queries and row['packets'] >= 40
        return row

    warmup = {tag: sample(tag) for tag in binaries}
    samples = []
    for i in range(rounds):
        order = ['baseline', 'after'] if i % 2 == 0 else ['after', 'baseline']
        row = {'round': i, 'order': order}
        for tag in order:
            row[tag] = sample(tag)
        samples.append(row)
    metrics = ['navigation_ns', 'seek_pcm_ns']
    medians = {tag: {metric: statistics.median(r[tag][metric] for r in samples) for metric in metrics} for tag in binaries}
    wins = {metric: sum(r['after'][metric] < r['baseline'][metric] for r in samples) for metric in metrics}
    ratios = {metric: medians['baseline'][metric] / medians['after'][metric] for metric in metrics}
    accepted = ratios['navigation_ns'] >= 1.2 and wins['navigation_ns'] >= 6 and medians['after']['seek_pcm_ns'] <= medians['baseline']['seek_pcm_ns'] * 1.1
    print(json.dumps({'scope': 'Memory-resident MACE MP4; navigation setup and seek-to-first-PCM costs are separate. Not whole-player latency.',
        'binary_sha256': {k: hashlib.sha256(v.read_bytes()).hexdigest() for k, v in binaries.items()},
        'workload_sha256': hashlib.sha256(workload.read_bytes()).hexdigest(),
        'queries': queries, 'rounds': rounds, 'warmup_excluded': warmup,
        'samples': samples, 'medians': medians, 'wins': wins, 'ratios_baseline_over_after': ratios,
        'criteria': {'navigation_speedup_minimum': 1.2, 'navigation_wins_minimum': 6, 'seek_pcm_maximum_regression_percent': 10},
        'criteria_met': accepted}, indent=2))
    if not accepted:
        raise SystemExit(1)


if __name__ == '__main__':
    main()
