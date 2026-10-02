#!/usr/bin/env python3
"""Source policy controls; no Cargo, FFmpeg or network required."""
import unittest
from check_native_dependencies import external_test_calls


class OrdinaryTestPolicy(unittest.TestCase):
    def test_reference_environment_and_literal_launches_are_rejected(self):
        for source in [
            'std::env::var_os("FVID_REFERENCE_FFMPEG")',
            'std::env::var("FFPROBE_PATH")',
            'env!("FFMPEG_BINARY")',
            'Command::new("ffmpeg")',
            'Command :: new ( "/opt/homebrew/bin/ffprobe" )',
            'std::process::Command::new("ffmpeg.exe")',
        ]:
            with self.subTest(source=source):
                self.assertEqual(external_test_calls("\n" + source), [2])

    def test_saved_oracles_and_own_cli_are_allowed(self):
        source = '''// Reference bytes were generated using ffmpeg.
include_bytes!("fixtures/ffmpeg-nearest.rgb");
Command::new(env!("CARGO_BIN_EXE_fvid"));
'''
        self.assertEqual(external_test_calls(source), [])


if __name__ == "__main__":
    unittest.main()
