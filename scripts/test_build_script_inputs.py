"""Exercise the actual linker-setup scripts in offline, dependency-free Cargo projects."""
import os
import json
from pathlib import Path
import subprocess
import tempfile
import time
import unittest

ROOT = Path(__file__).resolve().parents[1]
COUNTER = '''    std::io::Write::write_all(&mut std::fs::OpenOptions::new().create(true).append(true).open(std::path::PathBuf::from(std::env::var_os("OUT_DIR").unwrap()).join("input_watch_count")).unwrap(), b"x").unwrap();
'''


class BuildScriptInputs(unittest.TestCase):
    def cargo(self, project, legacy=False, prefix=None):
        env = dict(os.environ, CARGO_TARGET_DIR=str(project / 'target'))
        if prefix is not None:
            env['FVID_FFMPEG_PREFIX'] = str(prefix)
        command = ['cargo', 'check', '--offline', '--quiet', '--message-format=json', '--manifest-path', str(project / 'Cargo.toml')]
        if legacy:
            command += ['--features', 'legacy-ffmpeg']
        result = subprocess.run(command, cwd=ROOT, env=env, text=True, capture_output=True)
        self.assertEqual(result.returncode, 0, result.stderr)
        return [json.loads(line) for line in result.stdout.splitlines() if line.startswith('{')]

    def counter(self, project):
        paths = list((project / 'target/debug/build').glob('fvid-*/out/input_watch_count'))
        self.assertEqual(len(paths), 1, paths)
        return paths[0], paths[0].stat().st_size

    def test_only_declared_file_and_optional_prefix_changes_rerun_linker_setup(self):
        for relative, legacy in [('build.rs', False), ('crates/fvid-media/build.rs', False),
                                 ('crates/fvid-media/build.rs', True)]:
            with self.subTest(script=relative, legacy=legacy), tempfile.TemporaryDirectory(prefix='fvid-build-input-') as directory:
                project = Path(directory)
                (project / 'src').mkdir()
                (project / 'docs').mkdir()
                (project / 'Cargo.toml').write_text('[package]\nname="fvid"\nversion="0.0.0"\nedition="2024"\n[features]\nlegacy-ffmpeg=[]\n[workspace]\n')
                (project / 'src/main.rs').write_text('fn main() {}\n')
                source = (ROOT / relative).read_text()
                self.assertNotIn('fvid_build_watch_counter', source, 'temporary benchmark instrumentation must be removed')
                self.assertEqual(source.count('fn main() {\n'), 1)
                (project / 'build.rs').write_text(source.replace('fn main() {\n', 'fn main() {\n' + COUNTER, 1))
                marker = project / 'docs/unrelated.md'
                marker.write_text('initial\n')
                prefix = project / 'prefix-a'
                self.cargo(project, legacy, prefix)
                path, before = self.counter(project)
                time.sleep(1.1)
                marker.write_text('unrelated documentation change\n')
                self.cargo(project, legacy, prefix)
                self.assertEqual(self.counter(project)[1], before, 'documentation must not rerun linker setup')
                time.sleep(1.1)
                with (project / 'src/main.rs').open('a') as output:
                    output.write('// Rust source input changed\n')
                events = self.cargo(project, legacy, prefix)
                binary = [event for event in events if event.get('reason') == 'compiler-artifact' and 'bin' in event.get('target', {}).get('kind', [])]
                self.assertEqual(len(binary), 1)
                self.assertFalse(binary[0]['fresh'], 'Rust source edits must still recompile the target')
                self.assertEqual(self.counter(project)[1], before, 'Rust source edits do not change linker setup')
                time.sleep(1.1)
                with (project / 'build.rs').open('a') as output:
                    output.write('// declared script input changed\n')
                self.cargo(project, legacy, prefix)
                self.assertEqual(self.counter(project)[1], before + 1, 'the build script itself must remain tracked')
                if legacy:
                    prefix = project / 'prefix-b'
                    self.cargo(project, legacy, prefix)
                    self.assertEqual(self.counter(project)[1], before + 2, 'the optional linker prefix must remain tracked')
                    emitted = (path.parent.parent / 'output').read_text()
                    self.assertIn('cargo:rustc-link-search=native=' + str(prefix / 'lib'), emitted)

    def test_windows_stack_directive_remains_target_specific(self):
        with tempfile.TemporaryDirectory(prefix='fvid-build-stack-') as directory:
            executable = Path(directory) / ('build-script.exe' if os.name == 'nt' else 'build-script')
            result = subprocess.run(['rustc', '--edition=2024', str(ROOT / 'build.rs'), '-o', str(executable)], cwd=ROOT, text=True, capture_output=True)
            self.assertEqual(result.returncode, 0, result.stderr)
            flag = 'cargo:rustc-link-arg-bin=fvid=/STACK:8388608'
            for target in ['windows', 'macos', 'linux']:
                output = subprocess.check_output([str(executable)], env=dict(os.environ, CARGO_CFG_TARGET_OS=target), text=True)
                self.assertEqual(flag in output.splitlines(), target == 'windows')


if __name__ == '__main__':
    unittest.main()
