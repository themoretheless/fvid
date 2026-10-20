#!/usr/bin/env python3
"""Explicit paired native MIDI CPU measurements; never run by ordinary tests.

Both executables must be builds of examples/midi_render_probe.rs. Outputs are
created exclusively, so interrupted or repeated runs cannot silently mix data.
CPU includes process preparation/checksums/warmup; elapsed_ns is render-only.
"""
import argparse
import hashlib
import json
from pathlib import Path
import random
import resource
import statistics
import subprocess


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--before', required=True)
    parser.add_argument('--after', required=True)
    parser.add_argument('--iterations', required=True, type=int)
    parser.add_argument('--pairs', required=True, type=int)
    parser.add_argument('--output-prefix', required=True)
    parser.add_argument('--first', choices=['before', 'after'], default='before')
    parser.add_argument('--seed', type=int, default=1)
    parser.add_argument('inputs', nargs='+')
    args = parser.parse_args()
    if args.iterations < 1 or args.pairs < 1:
        parser.error('iterations and pairs must be positive')
    raw_path = Path(args.output_prefix + '.jsonl')
    summary_path = Path(args.output_prefix + '.json')
    if summary_path.exists():
        parser.error(f'{summary_path} already exists')
    raw_path.parent.mkdir(parents=True, exist_ok=True)
    rows = []
    expected = None
    with raw_path.open('x') as raw:
        for pair in range(args.pairs):
            order = [args.first, 'after' if args.first == 'before' else 'before']
            if pair % 2:
                order.reverse()
            for version in order:
                before = resource.getrusage(resource.RUSAGE_CHILDREN)
                result = json.loads(subprocess.check_output([
                    getattr(args, version), str(args.iterations), *args.inputs,
                ]))
                after = resource.getrusage(resource.RUSAGE_CHILDREN)
                row = dict(pair=pair, version=version,
                           process_user_seconds=after.ru_utime-before.ru_utime,
                           process_cpu_seconds=(after.ru_utime+after.ru_stime)
                           -(before.ru_utime+before.ru_stime), **result)
                raw.write(json.dumps(row) + '\n')
                raw.flush()
                control = {key: result[key] for key in [
                    'iterations', 'inputs', 'pcm_bytes', 'outside_timing_pcm_controls',
                ]}
                if expected is None:
                    expected = control
                assert control == expected, 'PCM checksum or workload counters changed'
                assert result['pcm_bytes'] == args.iterations * sum(
                    item['bytes'] for item in result['outside_timing_pcm_controls'])
                rows.append(row)
    summary = dict(pairs=args.pairs, iterations=args.iterations, first=args.first,
                   scope='Native MIDI CPU render only. Whole-process CPU includes '
                   'untimed preparation/checksums/warmup. Host load not isolated; '
                   'all outliers retained; no device or tail-latency claim.',
                   executables={version: getattr(args, version) for version in ['before', 'after']},
                   inputs={path: hashlib.sha256(Path(path).read_bytes()).hexdigest()
                           for path in args.inputs},
                   binary_sha256={version: hashlib.sha256(Path(getattr(args, version))
                                  .read_bytes()).hexdigest() for version in ['before', 'after']})
    for metric in ['elapsed_ns', 'process_user_seconds', 'process_cpu_seconds']:
        ratios = []
        for pair in range(args.pairs):
            before = next(r for r in rows if r['pair'] == pair and r['version'] == 'before')
            after = next(r for r in rows if r['pair'] == pair and r['version'] == 'after')
            ratios.append(after[metric] / before[metric])
        rng = random.Random(args.seed)
        boot = sorted(statistics.median(rng.choices(ratios, k=args.pairs))
                      for _ in range(10000))
        summary[metric] = dict(median_ratio=statistics.median(ratios),
                               bootstrap_median95=[boot[250], boot[9749]], ratios=ratios)
    with summary_path.open('x') as output:
        output.write(json.dumps(summary, indent=2) + '\n')
    print(json.dumps(summary))


if __name__ == '__main__':
    main()
