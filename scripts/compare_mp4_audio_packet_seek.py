#!/usr/bin/env python3
"""Paired ordered-packet navigation and analytic seek/PCM measurement."""
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
    assert queries > 0 and rounds == 7
    binaries = {'baseline': baseline.resolve(), 'after': after.resolve()}

    def sample(tag):
        out = subprocess.check_output([str(binaries[tag]), str(workload.resolve()), str(queries)], text=True, timeout=600)
        row = json.loads(out)
        assert row['queries'] == queries and row['packets'] >= 6
        return row

    warmup = {tag: sample(tag) for tag in binaries}
    samples = []
    for i in range(rounds):
        order = ['baseline', 'after'] if i % 2 == 0 else ['after', 'baseline']
        row = {'round': i, 'order': order}
        for tag in order:
            row[tag] = sample(tag)
        samples.append(row)
    metrics = ['open_ns', 'first_seek_ns', 'cold_direct_seek_ns', 'navigation_ns', 'seek_pcm_ns']
    medians = {tag: {key: statistics.median(r[tag][key] for r in samples) for key in metrics} for tag in binaries}
    wins = {key: sum(r['after'][key] < r['baseline'][key] for r in samples) for key in metrics}
    ratios = {key: medians['baseline'][key] / medians['after'][key] for key in metrics}
    guards = {key: medians['after'][key] <= medians['baseline'][key] * 1.1 for key in ['open_ns', 'first_seek_ns', 'cold_direct_seek_ns']}
    met = ratios['navigation_ns'] >= 1.2 and wins['navigation_ns'] >= 6 and ratios['seek_pcm_ns'] >= 1.1 and wins['seek_pcm_ns'] >= 6 and all(guards.values())
    print(json.dumps({'scope': 'Memory-resident authored160000-packet IMA4 workload; exact analytic PCM and timestamps. No GPU or whole-player claim.',
        'binary_sha256': {tag: hashlib.sha256(path.read_bytes()).hexdigest() for tag, path in binaries.items()},
        'workload_sha256': hashlib.sha256(workload.read_bytes()).hexdigest(), 'queries': queries, 'rounds': rounds,
        'warmup_excluded': warmup, 'samples': samples, 'medians': medians, 'wins': wins, 'ratios_baseline_over_after': ratios,
        'criteria': {'navigation_minimum_speedup': 1.2, 'seek_pcm_minimum_speedup': 1.1, 'minimum_wins_each': 6, 'opening_and_first_seek_regression_limit_percent': 10},
        'nonregression_guards': guards, 'criteria_met': met}, indent=2))
    if not met:
        raise SystemExit(1)


if __name__ == '__main__':
    main()
