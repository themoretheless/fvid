import hashlib
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch
import generate_matroska_duration_regressions as duration
import generate_matroska_tail_regressions as tail


class DurationFixtures(unittest.TestCase):
    def test_saved_presentation_tail_cases_regenerate_exactly(self):
        saved = tail.OUT
        with tempfile.TemporaryDirectory() as directory:
            target = Path(directory)
            with patch.object(tail, 'OUT', target):
                tail.main()
            manifest = json.loads((target / 'manifest.json').read_text())
            self.assertEqual(manifest, json.loads((saved / 'manifest.json').read_text()))
            self.assertEqual(len(manifest['cases']), 8)
            for name, digest in manifest['files'].items():
                data = (target / name).read_bytes()
                self.assertEqual(data, (saved / name).read_bytes())
                self.assertEqual(hashlib.sha256(data).hexdigest(), digest)

    def test_saved_duration_cases_regenerate_exactly(self):
        saved = duration.OUT
        with tempfile.TemporaryDirectory() as directory:
            target = Path(directory)
            with patch.object(duration, 'OUT', target):
                duration.main()
            manifest = json.loads((target / 'manifest.json').read_text())
            self.assertEqual(manifest, json.loads((saved / 'manifest.json').read_text()))
            self.assertEqual(len(manifest['cases']), 3)
            for case in manifest['cases']:
                data = (target / case['file']).read_bytes()
                self.assertEqual(data, (saved / case['file']).read_bytes())
                self.assertEqual(hashlib.sha256(data).hexdigest(), case['sha256'])


if __name__ == '__main__':
    unittest.main()
