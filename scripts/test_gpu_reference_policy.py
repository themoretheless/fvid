#!/usr/bin/env python3
"""Verify native validation and opt-in reference execution without external tools."""
import builtins
import pathlib
import sys
import tempfile
import types
import unittest
from unittest.mock import patch
import validate_gpu


class ReferencePolicy(unittest.TestCase):
    def test_default_validation_never_imports_reference_or_launches_ffmpeg(self):
        original_import = builtins.__import__
        calls = []

        def guarded_import(name, *args, **kwargs):
            if name == 'benchmark_gpu_reference':
                raise AssertionError('ordinary validation imported external benchmark')
            return original_import(name, *args, **kwargs)

        def success(command, **kwargs):
            calls.append(command)
            return types.SimpleNamespace(stdout=b'rustc test\n')

        with tempfile.TemporaryDirectory() as folder:
            report = pathlib.Path(folder) / 'report.json'
            with patch.object(sys, 'argv', ['validate_gpu.py', '--binary', 'synthetic', '--report', str(report)]), \
                 patch.object(validate_gpu, 'file_sha256', return_value='synthetic'), \
                 patch.object(validate_gpu, 'source_files', return_value=[]), \
                 patch.object(validate_gpu, 'success', side_effect=success), \
                 patch.object(validate_gpu, 'validate', return_value={'cpu': 'cpu'}), \
                 patch.object(builtins, '__import__', side_effect=guarded_import):
                validate_gpu.main()
            self.assertEqual(calls, [['rustc', '--version']])
            self.assertIn('"status": "passed"', report.read_text())
            self.assertNotIn('"ffmpeg"', report.read_text())

    def test_reference_benchmark_retains_both_external_commands_and_rounds(self):
        import benchmark_gpu_reference
        calls = []
        args = types.SimpleNamespace(binary='fvid', ffmpeg='reference-tool', device=0,
                                     resolutions=['720'], frames=1, rounds=1, report='unused')

        def success(command, **kwargs):
            calls.append(command)
            return types.SimpleNamespace(stdout=b'reference version\n', stderr=b'diagnostic')

        def make_input(path, *args):
            path.write_bytes(b'synthetic input')

        report = {}
        with tempfile.TemporaryDirectory() as folder, \
             patch.object(validate_gpu, 'success', side_effect=success), \
             patch.object(validate_gpu, 'make_input', side_effect=make_input), \
             patch.object(validate_gpu, 'save'):
            benchmark_gpu_reference.run_benchmarks(args, pathlib.Path(folder), {'cpu': 'cpu'}, report)
        self.assertEqual(calls[0], ['reference-tool', '-version'])
        self.assertEqual(len(calls), 13)  # version + 2 cases × 3 engines × warmup/round
        self.assertEqual(report['ffmpeg'], 'reference version')
        self.assertEqual(len(report['benchmarks']), 2)
        for record in report['benchmarks']:
            self.assertEqual(set(record['commands']), {'cpu', 'ffmpeg_default', 'ffmpeg_1thread'})
            self.assertEqual(len(record['round_orders']), 1)
            self.assertEqual(set(record['round_orders'][0]), set(record['commands']))
            self.assertIn('-filter_threads', record['commands']['ffmpeg_1thread'])
            self.assertNotIn('-filter_threads', record['commands']['ffmpeg_default'])
            self.assertTrue(all(len(samples) == 1 for samples in record['seconds'].values()))


if __name__ == '__main__':
    unittest.main()
