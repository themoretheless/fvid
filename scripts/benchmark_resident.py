#!/usr/bin/env python3
"""Full-CLI benchmarks with sample equality gates; no codec/driver-only claims."""
import argparse
import datetime
import hashlib
import json
import pathlib
import platform
import random
import re
import shutil
import statistics
import subprocess
import tempfile
import time
from validate_gpu import ROOT, make_input, ffmpeg_command, file_sha256


def run(command, output):
    result = subprocess.run(command, stdout=output, stderr=subprocess.PIPE, timeout=180)
    if result.returncode:
        raise RuntimeError(f'{command}: {result.stderr.decode(errors="replace")}')
    return result.stderr.decode(errors='replace')


def signatures(path):
    with path.open('rb') as source:
        header = source.readline()
        width = int(re.search(rb' W(\d+)', header)[1])
        height = int(re.search(rb' H(\d+)', header)[1])
        if not re.search(rb' C420(?:jpeg)?(?:\s|$)', header):
            raise ValueError(f'Unexpected format: {header!r}')
        length = width * height * 3 // 2
        frames = []
        while marker := source.readline():
            if marker != b'FRAME\n' and not marker.startswith(b'FRAME '):
                raise ValueError('Bad frame marker')
            payload = source.read(length)
            if len(payload) != length:
                raise ValueError('Truncated frame')
            frames.append(hashlib.sha256(payload).hexdigest())
        return dict(width=width, height=height, frame_sha256=frames)


def check_backend(engine, diagnostic, frames):
    if engine.startswith('metal_'):
        if 'backend=metal ' not in diagnostic:
            raise RuntimeError('Explicit Metal was not executed')
        if engine == 'metal_resident':
            for key, expected in [('uploads', frames), ('downloads', frames), ('filter_passes', frames * 3)]:
                match = re.search(rf'\b{key}=(\d+)', diagnostic)
                if not match or int(match[1]) != expected:
                    raise RuntimeError(f'Unexpected resident transfer count: {diagnostic}')


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--frames', type=int, default=60)
    parser.add_argument('--rounds', type=int, default=7)
    parser.add_argument('--report', default=str(ROOT / 'benchmarks/resident-benchmark.json'))
    args = parser.parse_args()
    if args.frames <= 0 or args.rounds < 3:
        parser.error('frames must be positive; at least three rounds required')
    binary = str(ROOT / 'target/release/fvid')
    ffmpeg = shutil.which('ffmpeg')
    if not ffmpeg:
        raise RuntimeError('FFmpeg missing')
    authored = [*ROOT.joinpath('src').rglob('*.rs'), *ROOT.joinpath('src').rglob('*.wgsl'),
                *ROOT.joinpath('crates/fvid-cuda/src').rglob('*.rs'), *ROOT.joinpath('crates/fvid-cuda/src').rglob('*.cu'),
                ROOT / 'Cargo.toml', ROOT / 'Cargo.lock', ROOT / 'crates/fvid-cuda/Cargo.toml',
                ROOT / 'scripts/validate_gpu.py', pathlib.Path(__file__)]
    report = dict(created_utc=datetime.datetime.now(datetime.timezone.utc).isoformat(), status='running',
                  platform=platform.platform(), machine=platform.machine(), binary_sha256=file_sha256(binary),
                  ffmpeg=ffmpeg, ffmpeg_sha256=file_sha256(ffmpeg),
                  ffmpeg_version=subprocess.check_output([ffmpeg, '-version']).decode(),
                  rustc=subprocess.check_output(['rustc', '-Vv']).decode(),
                  source_sha256={str(p.relative_to(ROOT)): file_sha256(p) for p in authored},
                  cargo_snapshots={str(p.relative_to(ROOT)): p.read_text() for p in [ROOT/'Cargo.toml', ROOT/'Cargo.lock']},
                  method=dict(frames=args.frames, rounds=args.rounds, warmups=1, seed=20260905,
                              timing='Whole CLI -> null sink; warm file cache; includes process/device/pipeline setup, read/upload/render/readback/serialization. No encode/decode, fsync or kernel-only timing.',
                              quality='Every frame of the exact timed input compared by decoded Y4M sample SHA256 before timing.',
                              ffmpeg_baseline='Default CPU crop/hflip/vflip; not hardware-accelerated FFmpeg.',
                              selection='Resident 3 passes; fused Metal/CPU use equivalent coordinates. No shader-only speed claim.'), cases=[])
    destination = pathlib.Path(args.report)
    def save(): destination.write_text(json.dumps(report, indent=2) + '\n')
    save()
    rng = random.Random(20260905)
    with tempfile.TemporaryDirectory(prefix='fvid-resident-benchmark-') as temporary:
        work = pathlib.Path(temporary)
        for width, height in [(1280, 720), (1920, 1080), (3840, 2160)]:
            source = work / 'input.y4m'
            make_input(source, width, height, '420', args.frames)
            input_sha256 = file_sha256(source)
            x, y = width // 8 // 2 * 2, height // 8 // 2 * 2
            cw, ch = width // 2, height // 2
            for case in ['crop_first', 'flip_first']:
                crop = (x, y, cw, ch)
                equivalent = (width - x - cw if case == 'flip_first' else x, y, cw, ch)
                fused = ['--crop', ':'.join(map(str, equivalent)), '--hflip', '--vflip']
                commands = {
                    'cpu_fused': [binary, str(source), '-', '--backend', 'cpu'] + fused,
                    'metal_fused': [binary, str(source), '-', '--backend', 'metal'] + fused,
                    'metal_resident': [binary, str(source), '-', '--backend', 'metal'] +
                        (['--hflip', '--then', '--crop', ':'.join(map(str, crop)), '--then', '--vflip'] if case == 'flip_first' else
                         ['--crop', ':'.join(map(str, crop)), '--then', '--hflip', '--then', '--vflip']),
                }
                for name, single in [('ffmpeg_default', False), ('ffmpeg_1thread', True)]:
                    command = ffmpeg_command(ffmpeg, source, '420', crop, True, True, single)
                    if case == 'flip_first':
                        command[command.index('-vf') + 1] = f'hflip,crop={cw}:{ch}:{x}:{y}:exact=1,vflip'
                    commands[name] = command
                reference = None
                diagnostics = {}
                for engine, command in commands.items():
                    out = work / 'verify.y4m'
                    with out.open('wb') as sink:
                        diagnostics[engine] = run(command, sink)
                    check_backend(engine, diagnostics[engine], args.frames)
                    actual = signatures(out)
                    if len(actual['frame_sha256']) != args.frames:
                        raise RuntimeError('Wrong output frame count')
                    if reference is not None and actual != reference:
                        raise RuntimeError(f'Output mismatch: {width}x{height} {case} {engine}')
                    reference = actual
                    out.unlink()
                for engine, command in commands.items():
                    check_backend(engine, run(command, subprocess.DEVNULL), args.frames)
                samples = {key: [] for key in commands}
                orders = []
                for _ in range(args.rounds):
                    order = list(commands)
                    rng.shuffle(order)
                    orders.append(order)
                    for engine in order:
                        start = time.perf_counter_ns()
                        diagnostic = run(commands[engine], subprocess.DEVNULL)
                        seconds = (time.perf_counter_ns() - start) / 1e9
                        check_backend(engine, diagnostic, args.frames)
                        samples[engine].append(seconds)
                medians = {key: statistics.median(values) for key, values in samples.items()}
                report['cases'].append(dict(resolution=f'{width}x{height}', case=case, input_sha256=input_sha256,
                                            output_signature=reference, commands=commands, diagnostics=diagnostics,
                                            seconds=samples, median_seconds=medians,
                                            min_seconds={k:min(v) for k,v in samples.items()},
                                            max_seconds={k:max(v) for k,v in samples.items()}, round_orders=orders))
                save()
                print(f'{width}x{height} {case}: ' + ', '.join(f'{key}={value*1000:.2f}ms' for key,value in medians.items()), flush=True)
            source.unlink()
    if any(file_sha256(ROOT / p) != digest for p,digest in report['source_sha256'].items()) or file_sha256(binary) != report['binary_sha256']:
        raise RuntimeError('Sources or binary changed during benchmark')
    report['status'] = 'passed'
    report['completed_utc'] = datetime.datetime.now(datetime.timezone.utc).isoformat()
    save()

if __name__ == '__main__':
    main()
