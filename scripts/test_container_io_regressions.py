"""Verify saved synthetic videos regenerate without external tools or network."""
import hashlib
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import generate_container_io_regressions as generator


class ContainerIoFixtures(unittest.TestCase):
    def test_saved_videos_and_manifests_regenerate_exactly(self):
        saved = generator.OUT
        with tempfile.TemporaryDirectory() as temporary:
            generated = Path(temporary)
            with patch.object(generator, "OUT", generated):
                generator.main()
            self.assertEqual(len(list(generated.iterdir())), 6)
            for path in generated.iterdir():
                with self.subTest(file=path.name):
                    self.assertEqual(path.read_bytes(), (saved / path.name).read_bytes())
                    if path.suffix == ".json":
                        manifest = json.loads(path.read_text())
                        video = (generated / manifest["file"]).read_bytes()
                        self.assertEqual(hashlib.sha256(video).hexdigest(), manifest["sha256"])


if __name__ == "__main__":
    unittest.main()
