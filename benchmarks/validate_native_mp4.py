#!/usr/bin/env python3
"""Differential packet-index benchmark. FFmpeg tools are reference-only here.

Run: python3 benchmarks/validate_native_mp4.py
Creates a temporary AVC+B-frames+AAC file and an HEVC file, then compares every
packet's position, size, media DTS/PTS, duration and sync flag against ffprobe.
"""
import csv
import io
import json
import re
from pathlib import Path
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[1]


def run(*args):
    return subprocess.run(args, cwd=ROOT, check=True, capture_output=True, text=True).stdout


def compare_slice_headers(path, native_stderr):
    native = [line.split(',')[1:] for line in native_stderr.splitlines()
              if line.startswith('AVC_SLICE,')]
    if not native:
        return
    trace = subprocess.run(['ffmpeg', '-v', 'verbose', '-i', str(path), '-map', '0:v',
                            '-c', 'copy', '-bsf:v', 'trace_headers', '-f', 'null', '-'],
                           check=True, capture_output=True, text=True).stderr
    records = []
    current = None
    for line in trace.splitlines():
        if '[trace_headers @' not in line:
            continue
        if 'Slice Header' in line:
            current = {}
            records.append(current)
            continue
        if any(marker in line for marker in ['Packet:', 'Sequence Parameter Set',
                                             'Picture Parameter Set', 'Supplemental Enhancement']):
            current = None
        match = re.search(r'\]\s+(\d+)\s+(\w+)\s+([01]+)\s+=\s+(-?\d+)', line)
        if match and current is not None:
            position, field, bits, value = match.groups()
            current[field] = int(value)
            current['end'] = int(position) + len(bits) - 8
    assert len(native) == len(records), (path, len(native), len(records))
    for actual, reference in zip(native, records):
        assert int(actual[0]) == reference['first_mb_in_slice']
        assert int(actual[1]) == reference['frame_num']
        assert actual[2] == ['P', 'B', 'I', 'Sp', 'Si'][reference['slice_type'] % 5]
        assert int(actual[3]) == reference['pic_parameter_set_id']
        assert int(actual[7]) == reference['slice_qp_delta']
        assert int(actual[6]) == reference['end'], (path, actual, reference)
    print(f'{path.name}: {len(native)} slice headers and entropy offsets match trace_headers')


def compare(binary, path):
    result = subprocess.run([str(binary), str(path)], cwd=ROOT, check=True,
                            capture_output=True, text=True)
    actual = list(csv.DictReader(io.StringIO(result.stdout)))
    compare_slice_headers(path, result.stderr)
    if path.name == 'avc-intra-cavlc.mp4':
        blocks = [int(line.split(',')[1]) for line in result.stderr.splitlines()
                  if line.startswith('AVC_INTRA_MACROBLOCKS,')]
        assert blocks == [24, 24, 24], blocks
        print(f'{path.name}: all {sum(blocks)} intra macroblocks parsed')
    avc_metadata = [line.split(',')[1:] for line in result.stderr.splitlines()
                    if line.startswith('AVC_SPS,')]
    if avc_metadata:
        streams = json.loads(run('ffprobe', '-v', 'error', '-show_streams',
                                 '-of', 'json', str(path)))['streams']
        reference_video = next(s for s in streams if s['codec_name'] == 'h264')
        for metadata in avc_metadata:
            assert tuple(map(int, metadata[:2])) == (reference_video['width'], reference_video['height'])
    expected = json.loads(run(
        'ffprobe', '-v', 'error', '-ignore_editlist', '1', '-show_packets',
        '-show_entries', 'packet=stream_index,pos,size,dts,pts,duration,flags',
        '-of', 'json', str(path)))['packets']
    expected.sort(key=lambda p: (p['stream_index'], int(p['pos'])))
    actual.sort(key=lambda p: (int(p['track']), int(p['offset'])))
    assert len(actual) == len(expected), (path, len(actual), len(expected))
    fields = [('track', 'stream_index'), ('offset', 'pos'), ('size', 'size'),
              ('dts', 'dts'), ('pts', 'pts'), ('duration', 'duration')]
    for a, b in zip(actual, expected):
        for ours, theirs in fields:
            assert int(a[ours]) == int(b[theirs]), (path, ours, a, b)
        assert bool(int(a['sync'])) == ('K' in b['flags']), (path, a, b)
    orders = [list(map(int,line.split(',')[1:])) for line in result.stderr.splitlines()
              if line.startswith('AVC_POC,')]
    if orders:
        # Fixtures use progressive x264 with one picture per sample and POC step 2.
        # Compare independently indexed presentation times within each IDR sequence.
        groups = []
        for track,sample,idr,poc in orders:
            if idr:
                groups.append([])
            packets = [p for p in expected if p['stream_index'] == track]
            packet = packets[sample]
            groups[-1].append((poc,int(packet['pts']),int(packet['duration'])))
        for group in groups:
            origin = min(pts for _,pts,_ in group)
            for poc,pts,duration in group:
                assert poc * duration == 2 * (pts-origin), (path,poc,pts,origin,duration)
        print(f'{path.name}: {len(orders)} POC values match presentation order')
    print(f'{path.name}: {len(actual)} packets match all seven fields')


def main():
    output = run('cargo', 'build', '--locked', '--offline', '--no-default-features',
                 '--example', 'mp4_packets', '--message-format=json')
    artifacts = [json.loads(line) for line in output.splitlines() if line.startswith('{')]
    binary = Path(next(a['executable'] for a in artifacts
                       if a.get('reason') == 'compiler-artifact'
                       and a.get('target', {}).get('name') == 'mp4_packets'
                       and a.get('executable')))
    with tempfile.TemporaryDirectory(prefix='fvid-mp4-reference-') as tmp:
        avc = Path(tmp) / 'avc-aac.mp4'
        hevc = Path(tmp) / 'hevc.mp4'
        cropped = Path(tmp) / 'avc-cropped-10bit.mp4'
        baseline = Path(tmp) / 'avc-intra-cavlc.mp4'
        run('ffmpeg', '-v', 'error', '-f', 'lavfi', '-i', 'testsrc2=size=64x64:rate=12',
            '-f', 'lavfi', '-i', 'sine=frequency=440:sample_rate=48000', '-t', '1',
            '-c:v', 'libx264', '-bf', '2', '-g', '12', '-c:a', 'aac', str(avc))
        run('ffmpeg', '-v', 'error', '-f', 'lavfi', '-i', 'testsrc2=size=64x64:rate=12',
            '-t', '1', '-c:v', 'libx265', '-x265-params', 'log-level=error:pools=1', str(hevc))
        run('ffmpeg', '-v', 'error', '-f', 'lavfi', '-i', 'testsrc2=size=66x50:rate=24',
            '-t', '0.5', '-vf', 'setsar=4/3,format=yuv420p10le', '-c:v', 'libx264',
            '-profile:v', 'high10', str(cropped))
        run('ffmpeg', '-v', 'error', '-f', 'lavfi', '-i', 'testsrc2=size=96x64:rate=3',
            '-t', '1', '-c:v', 'libx264', '-profile:v', 'baseline',
            '-x264-params', 'cabac=0:keyint=1', str(baseline))
        compare(binary, baseline)
        compare(binary, cropped)
        compare(binary, avc)
        compare(binary, hevc)


if __name__ == '__main__':
    main()
