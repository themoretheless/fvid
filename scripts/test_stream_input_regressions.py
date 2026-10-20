"""Stream-input fixture integrity without networking or external codecs."""
import hashlib
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import generate_hls_parser_regressions as hls
import generate_http_io_regressions as http
import generate_player_control_regressions as controls
import generate_matroska_clock_regressions as clocks


class StreamInputFixtures(unittest.TestCase):
    def test_matroska_clocks_regenerate_exactly(self):
        saved = clocks.OUT
        with tempfile.TemporaryDirectory() as directory:
            target = Path(directory)
            with patch.object(clocks, "OUT", target):
                clocks.main()
            manifest = json.loads((target / "manifest.json").read_text())
            self.assertEqual(manifest, json.loads((saved / "manifest.json").read_text()))
            for name, digest in manifest["files"].items():
                with self.subTest(file=name):
                    self.assertEqual((target / name).read_bytes(), (saved / name).read_bytes())
                    self.assertEqual(hashlib.sha256((target / name).read_bytes()).hexdigest(), digest)

    def test_player_control_reproducers_regenerate_exactly(self):
        saved = controls.OUT
        with tempfile.TemporaryDirectory() as directory:
            target = Path(directory)
            with patch.object(controls, "OUT", target):
                controls.main()
            manifest = json.loads((target / "manifest.json").read_text())
            self.assertEqual(manifest, json.loads((saved / "manifest.json").read_text()))
            for name, digest in manifest["files"].items():
                with self.subTest(file=name):
                    self.assertEqual((target / name).read_bytes(), (saved / name).read_bytes())
                    self.assertEqual(hashlib.sha256((target / name).read_bytes()).hexdigest(), digest)

    def test_hls_cases_regenerate_and_share_a_valid_synthetic_video(self):
        saved = hls.OUT
        with tempfile.TemporaryDirectory() as directory:
            target = Path(directory)
            with patch.object(hls, "OUT", target):
                hls.main()
            manifest = json.loads((target / "manifest.json").read_text())
            self.assertEqual(manifest, json.loads((saved / "manifest.json").read_text()))
            for name, digest in manifest["files"].items():
                with self.subTest(file=name):
                    self.assertEqual((target / name).read_bytes(), (saved / name).read_bytes())
                    self.assertEqual(hashlib.sha256((target / name).read_bytes()).hexdigest(), digest)

    def test_http_body_reproducer_regenerates(self):
        with tempfile.TemporaryDirectory() as directory:
            target = Path(directory)
            with patch.object(http, "OUT", target):
                http.main()
            for path in target.iterdir():
                self.assertEqual(path.read_bytes(), (http.OUT / path.name).read_bytes())
            manifest = json.loads((target / "http-interrupted-body.json").read_text())
            self.assertEqual(hashlib.sha256((target / manifest["file"]).read_bytes()).hexdigest(),
                             manifest["sha256"])


if __name__ == "__main__":
    unittest.main()
