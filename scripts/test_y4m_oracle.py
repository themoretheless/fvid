#!/usr/bin/env python3
"""Known-pixel tests for the independent validation oracle."""
import unittest
from y4m_oracle import transform


class OracleTests(unittest.TestCase):
    def test_crop_and_reflections_all_planes(self):
        for chroma, chroma_input, chroma_expected in [
            ("420", [10, 11], [11]),
            ("422", [10, 11, 12, 13], [13, 11]),
            ("444", list(range(10, 18)), [17, 16, 13, 12]),
        ]:
            with self.subTest(chroma=chroma):
                header = f"YUV4MPEG2 W4 H2 F30:1 Ip C{chroma}\n".encode()
                frame = bytes(range(8)) + bytes(chroma_input) + bytes(v + 20 for v in chroma_input)
                result = transform(header + b"FRAME\n" + frame, (2, 0, 2, 2), True, True)
                expected_header = header.replace(b"W4", b"W2")
                expected = bytes([7, 6, 3, 2]) + bytes(chroma_expected) + bytes(v + 20 for v in chroma_expected)
                self.assertEqual(result, expected_header + b"FRAME\n" + expected)

    def test_ordered_chain_keeps_metadata_and_every_frame(self):
        header = b"YUV4MPEG2 W4 H2 F24:1 Ip C444 XTEST=retained\n"
        frame = bytes(range(8)) * 3
        data = header + (b"FRAME Xtag=1\n" + frame) * 2
        actual = transform(transform(transform(data, horizontal=True), (0, 0, 2, 2)), vertical=True)
        expected = header.replace(b"W4", b"W2") + (b"FRAME Xtag=1\n" + bytes([7, 6, 3, 2]) * 3) * 2
        self.assertEqual(actual, expected)

    def test_invalid_geometry_and_truncated_planes(self):
        data = b"YUV4MPEG2 W4 H2 F30:1 Ip C420\nFRAME\n" + bytes(12)
        for crop in [(1, 0, 2, 2), (0, 0, 6, 2), (0, 0, 0, 2)]:
            with self.assertRaises(ValueError):
                transform(data, crop)
        with self.assertRaisesRegex(ValueError, "truncated"):
            transform(data[:-1])


if __name__ == "__main__":
    unittest.main()
