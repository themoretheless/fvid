import hashlib
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch
import generate_matroska_header_stripping_regressions as stripping


class HeaderStrippingFixtures(unittest.TestCase):
    def test_saved_header_stripping_cases_regenerate_exactly(self):
        saved = stripping.OUT
        with tempfile.TemporaryDirectory() as directory:
            target = Path(directory)
            with patch.object(stripping, 'OUT', target):
                stripping.main()
            manifest = json.loads((target / 'manifest.json').read_text())
            self.assertEqual(manifest, json.loads((saved / 'manifest.json').read_text()))
            self.assertEqual(len(manifest['cases']), 9)
            for name, digest in manifest['files'].items():
                data = (target / name).read_bytes()
                self.assertEqual(data, (saved / name).read_bytes())
                self.assertEqual(hashlib.sha256(data).hexdigest(), digest)


if __name__ == '__main__':
    unittest.main()
