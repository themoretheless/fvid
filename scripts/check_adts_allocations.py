"""Offline allocation regression, isolated from production allocator policy.

Builds the actual current libraries, links the measurement-only allocator harness,
and verifies the first single-block packet does not retain a duplicate payload.
No fixture generation, FFmpeg, network access or wall-clock assertions.
"""
import argparse
import json
from pathlib import Path
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[1]

def check(target):
    command = ['cargo', 'build', '--locked', '--offline', '--no-default-features',
               '--lib', '-p', 'fvid', '-p', 'fvid-media', '--target-dir', str(target),
               '--message-format=json-render-diagnostics']
    built = subprocess.run(command, cwd=ROOT, stdout=subprocess.PIPE, text=True, check=True)
    libraries = {}
    for line in built.stdout.splitlines():
        row = json.loads(line)
        if row.get('reason') == 'compiler-artifact' and row['target']['name'] in ['fvid', 'fvid_media']:
            for filename in row['filenames']:
                if filename.endswith('.rlib'):
                    libraries[row['target']['name']] = filename
    assert set(libraries) == {'fvid', 'fvid_media'}, libraries
    binary = target / 'adts-allocation-regression'
    command = ['rustc', '--edition=2024', '-O', str(ROOT/'scripts/bench_adts_allocations.rs'),
               '-L', f'dependency={target}/debug/deps', '-o', str(binary)]
    for name, library in libraries.items():
        command += ['--extern', f'{name}={library}']
    subprocess.run(command, cwd=ROOT, check=True)
    subprocess.run([str(binary), '--check'], cwd=ROOT, check=True)

if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--target-dir', type=Path, help='Explicit reusable target; retained after the check')
    args = parser.parse_args()
    if args.target_dir:
        check(args.target_dir.resolve())
    else:
        with tempfile.TemporaryDirectory(prefix='fvid-adts-allocation-') as directory:
            check(Path(directory))
