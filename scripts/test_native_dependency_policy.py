#!/usr/bin/env python3
"""Source policy controls; no Cargo, FFmpeg or network required."""
import unittest
from unittest import mock
import check_native_dependencies as guard
import pathlib
import tempfile
from check_native_dependencies import external_test_calls, external_python_calls, audit_native_validators, audit_fixture_generators, NATIVE_VALIDATORS


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


class NativeValidatorPolicy(unittest.TestCase):
    def root(self, folder):
        root = pathlib.Path(folder)
        (root / "scripts").mkdir()
        for name in NATIVE_VALIDATORS:
            (root / "scripts" / name).write_text('subprocess.run(["cargo", "test"])')
        return root

    def test_native_registry_is_audited_without_banning_explicit_benchmarks(self):
        with tempfile.TemporaryDirectory() as folder:
            root = self.root(folder)
            (root / "scripts/benchmark_media_reference.py").write_text('subprocess.run(["ffmpeg"])')
            paths, failures = audit_native_validators(root)
            self.assertEqual(len(paths), 6)
            self.assertEqual(failures, [])

    def test_external_validator_launch_is_rejected_with_file_and_line(self):
        with tempfile.TemporaryDirectory() as folder:
            root = self.root(folder)
            (root / "scripts/validate_media.py").write_text('\nsubprocess.run(["ffprobe"])')
            _, failures = audit_native_validators(root)
            self.assertEqual(len(failures), 1)
            self.assertIn("scripts/validate_media.py:2:", failures[0])

    def test_missing_or_malformed_validator_is_not_passing_evidence(self):
        for content in [None, "subprocess.run(["]:
            with self.subTest(content=content), tempfile.TemporaryDirectory() as folder:
                root = self.root(folder)
                path = root / "scripts/validate_media.py"
                if content is None:
                    path.unlink()
                else:
                    path.write_text(content)
                _, failures = audit_native_validators(root)
                self.assertEqual(len(failures), 1)
                self.assertIn("could not be audited", failures[0])


class FixtureDirectoryPolicy(unittest.TestCase):
    def test_nested_fixture_generator_cannot_launch_reference_tools(self):
        with tempfile.TemporaryDirectory() as folder:
            root=pathlib.Path(folder)
            (root/"scripts").mkdir()
            (root/"scripts/generate_safe.py").write_text('data = bytes([1, 2, 3])')
            fixtures=root/"tests/fixtures/playback-errors"
            fixtures.mkdir(parents=True)
            (fixtures/"generate_bad.py").write_text('\nsubprocess.run(["ffmpeg", "-i", "test.y4m"])')
            paths,failures=audit_fixture_generators(root)
            self.assertEqual(len(paths),2)
            self.assertEqual(len(failures),1)
            self.assertIn("tests/fixtures/playback-errors/generate_bad.py:2:",failures[0])

    def test_fixture_input_generation_and_saved_references_remain_allowed(self):
        with tempfile.TemporaryDirectory() as folder:
            root=pathlib.Path(folder)
            fixtures=root/"tests/fixtures/nested"
            fixtures.mkdir(parents=True)
            (fixtures/"generate.py").write_text('Path("ffmpeg-reference.raw").read_bytes()')
            _,failures=audit_fixture_generators(root)
            self.assertEqual(failures,[])


class ReferenceMarkerIsolation(unittest.TestCase):
    def test_reference_link_search_is_allowed_but_library_backend_and_linkage_are_rejected(self):
        with tempfile.TemporaryDirectory() as folder:
            root = pathlib.Path(folder)
            source = root / "crates/fvid-media/src"
            source.mkdir(parents=True)
            lib = source / "lib.rs"
            build = source.parent / "build.rs"
            lib.write_text('pub use owned_video_decode::decode_video;')
            build.write_text('println!("cargo:rustc-link-search=native=/reference/lib");')
            self.assertFalse(guard.library_uses_ffmpeg(root))
            for bad in ['include!("legacy.rs");', 'include!("av.rs");']:
                lib.write_text(bad)
                self.assertTrue(guard.library_uses_ffmpeg(root))
            lib.write_text('pub use owned_video_decode::decode_video;')
            for bad in ['println!("cargo:rustc-link-lib=dylib=avutil");', 'bindgen.header("libavcodec/avcodec.h")']:
                build.write_text(bad)
                self.assertTrue(guard.library_uses_ffmpeg(root))


class ProductionMediaPolicy(unittest.TestCase):
    def test_normal_audit_rejects_legacy_in_production_media(self):
        def graph(manifest, features, target, offline):
            return [], features == ["--no-default-features", "--features", "media"]

        with mock.patch("sys.argv", ["guard", "--offline", "--target", "test-target"]), \
             mock.patch.object(guard, "dependencies", side_effect=graph) as dependencies, \
             mock.patch.object(guard, "audit_ordinary_tests", return_value=([], [])), \
             mock.patch.object(guard, "audit_fixture_generators", return_value=([], [])), \
             mock.patch.object(guard, "audit_native_validators", return_value=([], [])), \
             mock.patch("builtins.print"):
            with self.assertRaisesRegex(SystemExit, "production media: FFmpeg dependency reached graph"):
                guard.main()
            self.assertEqual(dependencies.call_count, 11)
            self.assertTrue(any(
                call.args[1] == ["--no-default-features", "--features", "native-cuda"]
                for call in dependencies.call_args_list
            ))

    def test_all_features_graph_is_mandatory_and_rejects_ffmpeg_adapter(self):
        def graph(manifest, features, target, offline):
            return ({"ffmpeg-sys-next"}, False) if features == ["--all-features"] else ([], False)
        with mock.patch("sys.argv", ["guard", "--offline", "--target", "test-target"]), \
             mock.patch.object(guard, "dependencies", side_effect=graph), \
             mock.patch.object(guard, "audit_ordinary_tests", return_value=([], [])), \
             mock.patch.object(guard, "audit_fixture_generators", return_value=([], [])), \
             mock.patch.object(guard, "audit_native_validators", return_value=([], [])), \
             mock.patch("builtins.print"):
            with self.assertRaisesRegex(SystemExit, "all production features: FFmpeg dependency reached graph"):
                guard.main()

    def test_normal_audit_rejects_each_production_cuda_feature_without_opt_in(self):
        for feature, name in [("media-cuda", "production CUDA"), ("cuda-hw", "media library CUDA")]:
            with self.subTest(feature=feature):
                def graph(manifest, features, target, offline):
                    return [], features == ["--no-default-features", "--features", feature]
                with mock.patch("sys.argv", ["guard", "--offline", "--target", "test-target"]), \
                     mock.patch.object(guard, "dependencies", side_effect=graph), \
                     mock.patch.object(guard, "audit_ordinary_tests", return_value=([], [])), \
                     mock.patch.object(guard, "audit_fixture_generators", return_value=([], [])), \
                     mock.patch.object(guard, "audit_native_validators", return_value=([], [])), \
                     mock.patch("builtins.print"):
                    with self.assertRaisesRegex(SystemExit, name + ": FFmpeg dependency reached graph"):
                        guard.main()


if __name__ == "__main__":
    unittest.main()
