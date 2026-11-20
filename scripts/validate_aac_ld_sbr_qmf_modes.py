"""Optional offline reference-mode isolation on own zero-core LD-SBR movies.

Normal tests need neither FDK nor this tool. Complex PCM has a separately frozen
independent float oracle. External PCM16 comparisons are observations, without
fitted delay, gain normalization, skipped prefixes or a conformance verdict.
"""
import argparse
import hashlib
import json
from pathlib import Path
import re
from fdk_reference_source import verify_source_archive
from validate_aac_ld_sbr_fdk_pcm import metrics, run, samples
from validate_aac_ld_mps_fdk_reference import admission_controls

ROOT = Path(__file__).resolve().parents[1]
FIX = ROOT/'tests/fixtures/playback-errors/aac-ld-sbr-silent-harmonics'


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--native-probe', type=Path, required=True)
    parser.add_argument('--fdk-probe', type=Path, required=True)
    parser.add_argument('--fdk-archive', type=Path, required=True)
    parser.add_argument('--work', type=Path, required=True)
    parser.add_argument('--report', type=Path, required=True)
    args = parser.parse_args()
    args.native_probe = args.native_probe.resolve()
    args.fdk_probe = args.fdk_probe.resolve()
    if args.work.resolve().is_relative_to(ROOT/'tests/fixtures'):
        raise ValueError('external reference output must stay outside own fixtures')
    source = verify_source_archive(args.fdk_archive)
    args.work.mkdir(parents=True, exist_ok=True)
    args.report.parent.mkdir(parents=True, exist_ok=True)
    cases = json.loads((FIX/'manifest.json').read_text())['cases']
    if len(cases) != 24:
        raise ValueError('unexpected authored harmonic sweep membership')
    report = dict(complete=False, scope=__doc__, source_git_tree=source,
                  native_probe_sha256=sha(args.native_probe), fdk_probe_sha256=sha(args.fdk_probe),
                  fdk_driver_sha256=sha(ROOT/'scripts/support/aac_ld_mps_fdk_reference_probe.cpp'),
                  fixture_manifest_sha256=sha(FIX/'manifest.json'), cases=[])
    for c in cases:
        name = c['name']
        records = args.work/(name+'.records')
        native = args.work/(name+'.f32le')
        run([args.native_probe, 'packets', FIX/c['video'], records])
        native_log = run([args.native_probe, 'complex', FIX/c['video'], native])
        actual = samples(native, 'f')
        gold = samples(FIX/c['golden'], 'f')
        if len(actual) != len(gold) or len(actual) != c['samples']:
            raise ValueError('native/frozen sample geometry mismatch')
        own_error = max(abs(a-b) for a, b in zip(actual, gold))
        if own_error >= 1e-7:
            raise ValueError('native harmonic PCM differs from independent frozen oracle')
        comparisons, paths = {}, {}
        for mode in ['complex', 'lowpower', 'auto']:
            path = args.work/(name+'.'+mode+'.s16le')
            log = run([args.fdk_probe, records, path, '--qmf='+mode])
            geometry = re.search(r'rate=(\d+) channels=(\d+)', log)
            if (not geometry or tuple(map(int, geometry.groups())) != (c['rate'], 1)
                    or 'qmf_requested='+mode not in log):
                raise ValueError('reference QMF request or output geometry mismatch')
            fixed = samples(path, 'h')
            comparisons[mode] = dict(pcm_sha256=sha(path), stdout=log,
                                     full=metrics(actual, fixed, 1, 0))
            paths[mode] = path
        if paths['auto'].read_bytes() != paths['lowpower'].read_bytes():
            raise ValueError('pinned ordinary ELD automatic mode differs from explicit Low Power')
        entry = dict(name=name, input_sha256=sha(FIX/c['video']), records_sha256=sha(records),
                     native_pcm_sha256=sha(native), native_stdout=native_log,
                     independent_float_max_error=own_error, comparisons=comparisons)
        report['cases'].append(entry)
        args.report.write_text(json.dumps(report, indent=2)+'\n')
        print(name, 'complex RMSE', comparisons['complex']['full']['per_channel'][0]['rmse'], flush=True)
    # Pin the default CLI behavior and distinguish invalid-mode refusal from
    # codec refusal, using a valid independently authored input.
    default = args.work/'default.s16le'
    default_log = run([args.fdk_probe, records, default])
    if 'qmf_requested=complex' not in default_log or default.read_bytes() != paths['complex'].read_bytes():
        raise ValueError('reference default is not explicit complex QMF')
    import subprocess
    invalid = subprocess.run([str(args.fdk_probe), str(records), str(default), '--qmf=invalid'],
                             capture_output=True, text=True, timeout=30)
    if invalid.returncode == 0 or 'expected --qmf=complex|lowpower|auto' not in invalid.stderr:
        raise ValueError('invalid reference QMF mode was not refused specifically')
    report['default_complex_matches_explicit'] = True
    report['invalid_mode_error'] = invalid.stderr
    report['protocol_admission'] = admission_controls(args.fdk_probe.resolve(), records, args.work)
    report['complete'] = True
    args.report.write_text(json.dumps(report, indent=2)+'\n')


if __name__ == '__main__':
    main()
