#!/usr/bin/env python3
import unittest
from generate_sami_sidecar_regressions import generate, OUT

class Reproduction(unittest.TestCase):
    def test_authored_bytes(self):
        files = generate()
        self.assertEqual(set(files), {str(p.relative_to(OUT)) for p in OUT.rglob('*') if p.is_file()})
        for name, data in files.items():
            self.assertEqual(data, (OUT/name).read_bytes(), name)

if __name__ == '__main__':
    unittest.main()
