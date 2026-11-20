#!/usr/bin/env python3
"""Optional offline full-stream ISO ELD core origin check, outside ordinary tests.

Requires separately obtained original paired MP4/WAV and pinned ISO inventory.
No delay search, gain fitting, tail padding or reference regeneration. The
one-PCM24-LSB bound qualifies these two streams, not whole-codec conformance.
"""
import argparse
import hashlib
import json
import math
from pathlib import Path
import re
from compare_aac_reference_pcm import integer_wave
from validate_aac_ld_sbr_fdk_pcm import raw_movie, run, samples, sha
from validate_iso_eld_admission import INVENTORY_SHA256

PINS = {
    'er_eld1000np_24_ep0.mp4': '017173db6979bb2d9894a5ee7b29af6e',
    'er_eld1000np_48_ep0.mp4': 'b1ddc9429beb1807daa5e93b9629d29c',
    'er_eld1000np_24.wav': '369af3861d5b15a998d0872be392ced3',
    'er_eld1000np_48.wav': 'e9ce7319a37a9533f2f72ff8084289f5',
}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--inputs', type=Path, required=True)
    parser.add_argument('--inventory', type=Path, required=True)
    parser.add_argument('--native-probe', type=Path, required=True)
    parser.add_argument('--work', type=Path, required=True)
    parser.add_argument('--report', type=Path, required=True)
    args = parser.parse_args()
    inventory = args.inventory.read_bytes()
    if hashlib.sha256(inventory).hexdigest() != INVENTORY_SHA256:
        raise ValueError('original ISO inventory pin mismatch')
    members = {}
    for line in inventory.decode().splitlines():
        digest, name = line.split(None, 1)
        members.setdefault(Path(name).name, set()).add(digest)
    inputs, work = args.inputs.resolve(), args.work.resolve()
    if work == inputs or work.is_relative_to(inputs) or 'playback-errors' in work.parts:
        raise ValueError('work must be outside original inputs and authored fixtures')
    for name, digest in PINS.items():
        if digest not in members.get(name, set()):
            raise ValueError(f'original member absent from inventory: {name}')
        if hashlib.md5((inputs/name).read_bytes()).hexdigest() != digest:
            raise ValueError(f'original member checksum mismatch: {name}')
    work.mkdir(parents=True, exist_ok=True)
    args.report.parent.mkdir(parents=True, exist_ok=True)
    report = dict(scope='Two original ELD core streams; complete PCM; zero lag; not full codec conformance',
                  complete=False, native_probe_sha256=sha(args.native_probe),
                  inventory_sha256=INVENTORY_SHA256, cases=[])
    for rate in [24, 48]:
        name = f'er_eld1000np_{rate}'
        source, reference = inputs/(name+'_ep0.mp4'), inputs/(name+'.wav')
        raw, edits = raw_movie(source.read_bytes())
        movie = work/(name+'.raw.mp4')
        movie.write_bytes(raw)
        original_records, raw_records = work/(name+'.original.records'), work/(name+'.raw.records')
        run([args.native_probe, 'packets', source, original_records])
        run([args.native_probe, 'packets', movie, raw_records])
        if original_records.read_bytes() != raw_records.read_bytes():
            raise ValueError('hiding edit box changed original ASC/AUs')
        pcm = work/(name+'.f32le')
        log = run([args.native_probe, 'complex', movie, pcm])
        channels, hz, bits, gold = integer_wave(reference.read_bytes())
        actual = samples(pcm, 'f')
        if (channels, hz, bits) != (1, rate*1000, 24) or len(actual) != len(gold):
            raise ValueError('original reference/native PCM geometry mismatch')
        geometry = re.fullmatch(r'mode=Complex packets=(\d+) rate=(\d+) channels=(\d+) scalars=(\d+)\s*', log)
        if (geometry is None or tuple(map(int, geometry.groups())) !=
                (len(gold)//512, hz, channels, len(gold))):
            raise ValueError('native decoder reported inconsistent stream geometry')
        square = maximum = first = last = 0.0
        for i, value in enumerate(actual):
            if not math.isfinite(value):
                raise ValueError('non-finite native PCM')
            delta = abs(value-gold[i]/8388608.0)
            square += delta*delta
            maximum = max(maximum, delta)
            if i < 128:
                first = max(first, delta)
            if i >= len(gold)-128:
                last = max(last, delta)
        row = dict(name=name, rate=hz, channels=channels, reference_bits=bits,
                   frames=len(gold)//channels, explicit_lag=0, hidden_edits=edits,
                   source_sha256=sha(source), reference_sha256=sha(reference),
                   records_sha256=sha(raw_records), native_pcm_sha256=sha(pcm),
                   rmse=math.sqrt(square/len(gold)), max_abs_error=maximum,
                   first_quarter_max_error=first, final_quarter_max_error=last,
                   bound=1/8388608, passed=maximum <= 1/8388608, native_log=log.strip())
        report['cases'].append(row)
        args.report.write_text(json.dumps(report, indent=2)+'\n')
        print(name, row['passed'], row['max_abs_error'], flush=True)
    report.update(complete=True, passed=all(c['passed'] for c in report['cases']))
    args.report.write_text(json.dumps(report, indent=2)+'\n')
    return 0 if report['passed'] else 1


if __name__ == '__main__':
    raise SystemExit(main())
