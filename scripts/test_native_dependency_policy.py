#!/usr/bin/env python3
"""Source policy controls; no Cargo, FFmpeg or network required."""
import unittest
from check_native_dependencies import external_test_calls, external_python_calls


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


class FixtureGeneratorPolicy(unittest.TestCase):
    def test_literal_launches_and_environment_hooks_are_rejected(self):
        for source in [
            'subprocess.run(["ffmpeg", "-version"])',
            'execute(("/usr/bin/ffprobe", "input"))',
            'subprocess.Popen("ffmpeg.exe")',
            'os.getenv("FVID_REFERENCE_FFMPEG")',
            'os.environ.get("FFPROBE_PATH")',
            'os.environ["FFMPEG_BINARY"]',
            'shutil.which("ffmpeg")',
        ]:
            with self.subTest(source=source):
                self.assertEqual(external_python_calls("\n" + source), [2])

    def test_comments_oracles_and_own_tools_are_allowed(self):
        self.assertEqual(external_python_calls('''
"""Previously generated with FFmpeg."""
# subprocess.run(["ffmpeg"])
Path("ffmpeg-reference.rgb").read_bytes()
subprocess.run(["fvid", "input.y4m"])
os.environ.get("FVID_BINARY")
'''), [])

    def test_malformed_source_is_not_treated_as_a_passing_audit(self):
        with self.assertRaises(SyntaxError):
            external_python_calls("subprocess.run([");


if __name__ == "__main__":
    unittest.main()
