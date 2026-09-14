#!/usr/bin/env python3
"""Qualify resident CLI chains on a real GPU; do not rewrite prior snapshots."""
import argparse
import datetime
import hashlib
import json
import pathlib
import re
import subprocess
import tempfile
from validate_gpu import ROOT, make_input, signature, ffmpeg_command, file_sha256


def execute(command, data=None):
    result = subprocess.run(command, input=data, capture_output=True, timeout=180)
    if result.returncode:
        raise RuntimeError(f"{command}: {result.stderr.decode(errors='replace')}")
    return result


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--backend', default='metal', choices=['metal', 'cuda', 'vulkan', 'dx12', 'gl'])
    parser.add_argument('--report', default=str(ROOT / 'benchmarks/resident-results.json'))
    args = parser.parse_args()
    binary = str(ROOT / 'target/release/fvid')
    api_test = None
    if args.backend in ('metal', 'cuda'):
        command = ['cargo', 'test', '--offline', '--test', 'resident',
                   f'{args.backend}_resident_chain', '--', '--ignored', '--nocapture']
        result = execute(command)
        api_test = dict(command=command, stdout=result.stdout.decode(), stderr=result.stderr.decode(), exit_code=0)
    records = []
    cases = [(8, 6, '420'), (8, 6, '422'), (9, 7, '444'), (65, 17, '444'),
             (638, 480, '420'), (1920, 1080, '420'), (3840, 2160, '420')]
    with tempfile.TemporaryDirectory(prefix='fvid-resident-') as temporary:
        folder = pathlib.Path(temporary)
        for width, height, chroma in cases:
            source = folder / 'input.y4m'
            make_input(source, width, height, chroma, 3)
            crop = (2, 2, width - 4, height - 4)
            crop_arg = ':'.join(map(str, crop))
            command = [binary, str(source), '-', '--backend', args.backend,
                       '--crop', crop_arg, '--then', '--hflip', '--then', '--vflip']
            gpu = execute(command)
            actual = signature(gpu.stdout)
            cpu = execute([binary, str(source), '-', '--backend', 'cpu', '--crop', crop_arg, '--hflip', '--vflip'])
            reference = execute(ffmpeg_command('ffmpeg', source, chroma, crop, True, True))
            assert actual == signature(cpu.stdout) == signature(reference.stdout)
            diagnostics = gpu.stderr.decode()
            assert f'backend={args.backend} ' in diagnostics
            stats = {key: int(value) for key, value in re.findall(r'\b(uploads|downloads|upload_bytes|download_bytes|filter_passes)=(\d+)', diagnostics)}
            assert stats['uploads'] == stats['downloads'] == 3
            assert stats['filter_passes'] == 9
            records.append(dict(width=width, height=height, format=chroma, stages=3,
                                input_sha256=file_sha256(source), output=actual,
                                transfers=stats, diagnostic=diagnostics, command=command))
        # Order-sensitive crop after reflection cannot be treated as the old
        # single Transform, whose crop always occurs before reflection.
        source = folder / 'ordered.y4m'
        make_input(source, 16, 12, '420', 3)
        command = [binary, str(source), '-', '--backend', args.backend,
                   '--hflip', '--then', '--crop', '2:2:8:6', '--then', '--vflip']
        gpu = execute(command)
        first = execute([binary, str(source), '-', '--backend', 'cpu', '--hflip'])
        second = execute([binary, '-', '-', '--backend', 'cpu', '--crop', '2:2:8:6'], first.stdout)
        third = execute([binary, '-', '-', '--backend', 'cpu', '--vflip'], second.stdout)
        reference = execute(['ffmpeg', '-nostdin', '-v', 'error', '-i', str(source),
                             '-vf', 'hflip,crop=8:6:2:2:exact=1,vflip', '-c:v', 'rawvideo',
                             '-pix_fmt', 'yuv420p', '-f', 'yuv4mpegpipe', '-'])
        assert signature(gpu.stdout) == signature(third.stdout) == signature(reference.stdout)
        records.append(dict(width=16, height=12, format='420', stages=3, order='hflip,crop,vflip',
                            input_sha256=file_sha256(source), output=signature(gpu.stdout),
                            diagnostic=gpu.stderr.decode(), command=command))
        # The valid first frame must not be published if the second is truncated.
        broken = folder / 'broken.y4m'
        broken.write_bytes(b'YUV4MPEG2 W8 H6 C420\nFRAME\n' + bytes(72) + b'FRAME\n\x01')
        target = folder / 'must-not-exist.y4m'
        failed = subprocess.run([binary, str(broken), str(target), '--backend', args.backend,
                                 '--hflip', '--then', '--vflip'], capture_output=True, timeout=60)
        assert failed.returncode != 0 and not target.exists()
        assert not list(folder.glob('.fvid-*.tmp'))
        failure = dict(exit_code=failed.returncode, diagnostic=failed.stderr.decode(), unpublished=True)
    authored = [*ROOT.joinpath('src').rglob('*.rs'), *ROOT.joinpath('src').rglob('*.wgsl'),
                *ROOT.joinpath('crates/fvid-cuda/src').rglob('*.rs'), *ROOT.joinpath('crates/fvid-cuda/src').rglob('*.cu'),
                ROOT/'Cargo.toml', ROOT/'Cargo.lock', ROOT/'crates/fvid-cuda/Cargo.toml',
                ROOT/'tests/resident.rs', pathlib.Path(__file__)]
    report = dict(created_utc=datetime.datetime.now(datetime.timezone.utc).isoformat(), backend=args.backend,
                  binary_sha256=file_sha256(binary), source_sha256={str(p.relative_to(ROOT)): file_sha256(p) for p in authored},
                  qualification='Actual GPU execution; API transfer counters, not a driver trace. No codec surface interop.',
                  cases=records, failure_check=failure, resident_api_test=api_test)
    target = pathlib.Path(args.report)
    target.write_text(json.dumps(report, indent=2) + '\n')
    print(f'{len(records)} resident CLI cases match CPU and FFmpeg; atomic failure passed; report={target}')


if __name__ == '__main__':
    main()
