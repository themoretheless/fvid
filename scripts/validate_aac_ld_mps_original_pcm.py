#!/usr/bin/env python3
"""Explicit offline original ISO LD-MPS waveform audit, never an ordinary test.

Requires separately fetched original vectors and the pinned ISO MD5 inventory.
Uses the owned probe and strict signed-WAVE comparator. No fitting, FFmpeg,
foreign decoder, network or reference-media copies in repository fixtures.
"""
import argparse
import hashlib
import json
from pathlib import Path
import re
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[1]
INVENTORY_SHA256 = '865521883bbc19db177c71c00fcb57bd3619a8ae73a05a21532c3fc22cc13e92'
NAMES = ['er_ad_ldmps_tree_5151', 'er_eld_ldmps_tree_5151',
         'er_eld_ldmps_ldqmf_1', 'er_eld_ldmps_ldqmf_2']


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--vectors-dir', type=Path, required=True)
    parser.add_argument('--inventory', type=Path, required=True)
    parser.add_argument('--native-probe', type=Path, required=True)
    parser.add_argument('--output-dir', type=Path, required=True)
    parser.add_argument('--report', type=Path, required=True)
    args = parser.parse_args()
    out = args.output_dir.resolve()
    if (args.report.resolve().is_relative_to(args.vectors_dir.resolve())
            or args.report.resolve() in [args.inventory.resolve(), args.native_probe.resolve()]):
        raise ValueError('report must not overwrite original inputs or tools')
    if (out.is_relative_to(args.vectors_dir.resolve())
            or out.is_relative_to(ROOT/'tests/fixtures')):
        raise ValueError('research outputs must be separate from originals and fixtures')
    inventory = args.inventory.read_bytes()
    if hashlib.sha256(inventory).hexdigest() != INVENTORY_SHA256:
        raise ValueError('ISO checksum inventory differs from the qualified pin')
    sums = {}
    for line in inventory.decode().splitlines():
        digest, name = line.split(None, 1)
        sums[name.lstrip('./')] = digest
    inputs = []
    for name in NAMES:
        for folder, ext in [('compressedMp4', '.mp4'), ('referencesWav', '.wav')]:
            path = args.vectors_dir/(name+ext)
            data = path.read_bytes()
            digest = hashlib.md5(data).hexdigest()
            if digest != sums[folder+'/'+name+ext]:
                raise ValueError('original ISO input checksum mismatch: '+path.name)
            inputs.append(dict(name=path.name, iso_md5=digest,
                               sha256=hashlib.sha256(data).hexdigest(), bytes=len(data)))
    out.mkdir(parents=True, exist_ok=True)
    cases = []
    for name in NAMES:
        for mode in ['real', 'complex']:
            pcm = out/(name+'.'+mode+'.f32le')
            decoded = subprocess.run([str(args.native_probe.resolve()), mode,
                                      str(args.vectors_dir/(name+'.mp4')), str(pcm)],
                                     capture_output=True, text=True, check=True)
            compared = subprocess.run([sys.executable,
                                      str(ROOT/'scripts/compare_aac_reference_pcm.py'),
                                      str(args.vectors_dir/(name+'.wav')), str(pcm)],
                                     capture_output=True, text=True, check=True)
            metrics = json.loads(compared.stdout)
            geometry = re.search(r'rate=(\d+) channels=(\d+) scalars=(\d+)', decoded.stdout)
            if geometry is None or tuple(map(int, geometry.groups())) != (
                    metrics['rate'], metrics['channels'],
                    metrics['input_frames']*metrics['channels']):
                raise ValueError('native and original WAVE output geometry differ')
            cases.append(dict(name=name, mode=mode, native_output=decoded.stdout,
                              native_stderr=decoded.stderr, comparison=metrics))
            print(name, mode, 'RMS', max(c['rmse'] for c in metrics['per_channel']),
                  'max', max(c['max_abs_error'] for c in metrics['per_channel']), flush=True)
    if any(sha(args.vectors_dir/c['name']) != c['sha256'] for c in inputs):
        raise ValueError('original inputs changed during the audit')
    report = dict(complete=True, conformance_verdict=None, explicit_lag=0,
                  delay_search=False, gain_fit=False,
                  scope='Four original ISO LD-MPS inputs; both owned decorrelation modes. '
                        'Full stream and exact geometry. Numerical evidence, not an '
                        'accuracy-category or complete-profile conformance verdict.',
                  inventory_sha256=INVENTORY_SHA256,
                  native_probe_sha256=sha(args.native_probe),
                  generator_sha256=sha(Path(__file__)), inputs=inputs, cases=cases)
    args.report.parent.mkdir(parents=True, exist_ok=True)
    args.report.write_text(json.dumps(report, indent=2)+'\n')


if __name__ == '__main__':
    main()
