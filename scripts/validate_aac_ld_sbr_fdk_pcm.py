#!/usr/bin/env python3
"""Offline, optional original ISO LD-SBR PCM comparison; no conformance verdict.

Requires the separately built original FDK probe and pinned source archive.
Production and ordinary tests never invoke this tool. Original ASC/AUs are
verified unchanged when edit boxes are hidden for raw-decoder comparison.
Every full stream is compared at zero lag; the later window is diagnostic only.
"""
import argparse
import array
import hashlib
import json
from pathlib import Path
import re
import struct
import subprocess
import sys
from validate_iso_eld_admission import INVENTORY, INVENTORY_SHA256, selected
from fdk_reference_source import COMMIT, verify_source_archive

ROOT = Path(__file__).resolve().parents[1]


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def boxes(data, start, end):
    while start < end:
        if end-start < 8:
            raise ValueError('truncated MP4 research box')
        size, tag = struct.unpack_from('>I4s', data, start)
        header = 8
        if size == 1:
            if end-start < 16:
                raise ValueError('truncated extended research box')
            size = struct.unpack_from('>Q', data, start+8)[0]
            header = 16
        if size == 0:
            size = end-start
        if not header <= size <= end-start:
            raise ValueError('invalid MP4 research box bound')
        yield tag, start, start+header, start+size
        start += size


def raw_movie(data):
    result = bytearray(data)
    tracks = edits = 0
    for tag, _, first, end in boxes(data, 0, len(data)):
        if tag != b'moov':
            continue
        for tag, _, first, end in boxes(data, first, end):
            if tag != b'trak':
                continue
            children = list(boxes(data, first, end))
            audio = False
            for kind, _, a, b in children:
                if kind == b'mdia':
                    for kind, _, a, b in boxes(data, a, b):
                        if kind == b'hdlr' and b-a >= 12 and data[a+8:a+12] == b'soun':
                            audio = True
            if audio:
                tracks += 1
                for kind, at, _, _ in children:
                    if kind == b'edts':
                        result[at+4:at+8] = b'free'
                        edits += 1
    if tracks != 1:
        raise ValueError('reference comparison requires exactly one audio track')
    return result, edits


def run(args):
    result = subprocess.run([str(v) for v in args], capture_output=True, text=True, timeout=180)
    if result.returncode:
        raise RuntimeError(f'reference command failed: {result.stderr}')
    return result.stdout


def samples(path, kind):
    result = array.array(kind, path.read_bytes())
    if sys.byteorder != 'little':
        result.byteswap()
    return result


def metrics(actual, reference, channels, start):
    if len(actual) != len(reference) or len(actual) % channels or len(actual)//channels <= start:
        raise ValueError('incompatible raw PCM comparison geometry')
    rows = []
    frames = len(actual)//channels-start
    for channel in range(channels):
        error = maximum = 0.0
        for i in range(start*channels+channel, len(actual), channels):
            value = actual[i]
            if not (-float('inf') < value < float('inf')):
                raise ValueError('non-finite native PCM')
            delta = value-reference[i]/32768.0
            error += delta*delta
            maximum = max(maximum, abs(delta))
        rows.append(dict(channel=channel, compared_frames=frames, rmse=(error/frames)**0.5,
                         max_abs_error=maximum))
    return dict(from_frame=start, explicit_lag=0, per_channel=rows)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--inputs', type=Path, required=True)
    parser.add_argument('--native-probe', type=Path, required=True)
    parser.add_argument('--fdk-probe', type=Path, required=True)
    parser.add_argument('--fdk-archive', type=Path, required=True)
    parser.add_argument('--work', type=Path, required=True)
    parser.add_argument('--report', type=Path, required=True)
    parser.add_argument('--fdk-qmf', choices=['complex', 'lowpower', 'auto'], default='complex',
                        help='explicit reference renderer; native PCM always uses complex QMF')
    args = parser.parse_args()
    args.native_probe = args.native_probe.resolve()
    args.fdk_probe = args.fdk_probe.resolve()
    source_pin = verify_source_archive(args.fdk_archive)
    inputs, work = args.inputs.resolve(), args.work.resolve()
    if work == inputs or work.is_relative_to(inputs) or 'playback-errors' in work.parts:
        raise ValueError('reference work must be outside original inputs and authored fixtures')
    data = (inputs/INVENTORY).read_bytes()
    if hashlib.sha256(data).hexdigest() != INVENTORY_SHA256:
        raise ValueError('original ISO checksum inventory pin mismatch')
    names = {}
    for line in data.decode().splitlines():
        digest, path = line.split(None, 1)
        path = path.lstrip('./')
        if path.startswith('compressedMp4/') and selected(path, 'eld-sbr'):
            name = Path(path).name
            if name in names:
                raise ValueError('duplicate ISO LD-SBR member')
            names[name] = digest
    if len(names) != 26:
        raise ValueError('unexpected original ISO LD-SBR membership')
    for name, md5 in names.items():
        if hashlib.md5((inputs/name).read_bytes()).hexdigest() != md5:
            raise ValueError(f'original ISO member checksum mismatch: {name}')
    work.mkdir(parents=True, exist_ok=True)
    args.report.parent.mkdir(parents=True, exist_ok=True)
    report = dict(scope='Native f32 versus original FDK PCM16;raw compressed tracks,zero lag;not conformance',
                  complete=False, source_archive_sha256=source_pin['archive_sha256'],
                  source_git_tree=source_pin, fdk_source_commit=COMMIT, fdk_qmf_requested=args.fdk_qmf,
                  fdk_driver_sha256=sha(ROOT/'scripts/support/aac_ld_mps_fdk_reference_probe.cpp'),
                  fdk_probe_sha256=sha(args.fdk_probe), native_probe_sha256=sha(args.native_probe),
                  iso_inventory_sha256=INVENTORY_SHA256, cases=[])
    for name in sorted(names):
        source = inputs/name
        raw, count = raw_movie(source.read_bytes())
        stripped = work/name
        stripped.write_bytes(raw)
        records, original = work/(name+'.records'), work/(name+'.original.records')
        run([args.native_probe, 'packets', source, original])
        run([args.native_probe, 'packets', stripped, records])
        if original.read_bytes() != records.read_bytes():
            raise ValueError('hiding edits changed original ASC/AUs')
        native, fixed = work/(name+'.f32le'), work/(name+'.s16le')
        native_log = run([args.native_probe, 'complex', stripped, native])
        fdk_log = run([args.fdk_probe, records, fixed, '--qmf='+args.fdk_qmf])
        if 'qmf_requested='+args.fdk_qmf not in fdk_log:
            raise ValueError('FDK probe did not confirm requested QMF mode')
        rate, channels = map(int, re.search(r'rate=(\d+) channels=(\d+)', fdk_log).groups())
        actual, reference = samples(native, 'f'), samples(fixed, 'h')
        geometry = tuple(map(int, re.search(r'rate=(\d+) channels=(\d+)', native_log).groups()))
        native_count = int(re.search(r'scalars=(\d+)', native_log).group(1))
        reference_count = int(re.search(r'scalars=(\d+)', fdk_log).group(1))
        if geometry != (rate, channels) or native_count != len(actual) or reference_count != len(reference):
            raise ValueError('declared native/reference PCM geometry mismatch')
        if re.search(r'packets=(\d+)', native_log).group(1) != re.search(r'packets=(\d+)', fdk_log).group(1):
            raise ValueError('native/reference AU count mismatch')
        row = dict(name=name, input_sha256=sha(source), iso_md5=names[name], hidden_edit_boxes=count,
                   original_records_sha256=sha(original), native_stdout=native_log, fdk_stdout=fdk_log,
                   rate=rate, channels=channels, sample_frames=len(actual)//channels,
                   native_pcm_sha256=sha(native), fdk_pcm_sha256=sha(fixed),
                   native_peak=max(map(abs, actual)), fdk_peak=max(map(abs, reference))/32768.0,
                   native_full_scale_overshoots=sum(abs(v) > 1.0 for v in actual),
                   full=metrics(actual, reference, channels, 0),
                   later_diagnostic=metrics(actual, reference, channels, 2048))
        report['cases'].append(row)
        args.report.write_text(json.dumps(report, indent=2)+'\n')
        print(name, row['full'], flush=True)
    report['complete'] = True
    args.report.write_text(json.dumps(report, indent=2)+'\n')


if __name__ == '__main__':
    main()
