"""Synthetic WAVE reference-reader tests; no media generator/network/process."""
import struct
import unittest
from compare_aac_reference_pcm import integer_wave, pcm16_wave


def chunk(tag, data):
    return tag+struct.pack('<I', len(data))+data+b'\0'*(len(data) % 2)


def wave(depth, values, channels=1, extensible=False, valid=None, payload=None, duplicates=None):
    stride = channels*(depth//8)
    fmt = struct.pack('<HHIIHH', 0xfffe if extensible else 1, channels, 24000,
                      24000*stride, stride, depth)
    if extensible:
        fmt += struct.pack('<HHI', 22, depth if valid is None else valid, 0)
        fmt += bytes.fromhex('0100000000001000800000aa00389b71')
    if payload is None:
        payload = b''.join(v.to_bytes(depth//8, 'little', signed=True) for v in values)
    parts = [chunk(b'fmt ', fmt), chunk(b'data', payload)]
    if duplicates == 'fmt': parts.insert(1, parts[0])
    if duplicates == 'data': parts.append(parts[-1])
    body = b'WAVE'+b''.join(parts)
    return b'RIFF'+struct.pack('<I', len(body))+body


class ReferencePcmTests(unittest.TestCase):
    def test_pcm16_signed_extremes_preserve_existing_strict_contract(self):
        values = [-32768, -1, 0, 1, 32767]
        for ext in [False, True]:
            channels, rate, data = pcm16_wave(wave(16, values, extensible=ext))
            self.assertEqual((channels, rate, list(data)), (1, 24000, values))

    def test_packed_pcm24_sign_extension_and_extensible_stereo(self):
        values = [-8388608, -1, 0, 1, 8388607, 0x010203]
        for ext in [False, True]:
            ch, rate, depth, data = integer_wave(wave(24, values, 2, ext))
            self.assertEqual((ch, rate, depth, list(data)), (2, 24000, 24, values))

    def test_pcm32_signed_extremes_are_not_narrowed_to_pcm16(self):
        values = [-2147483648, -1, 0, 1, 2147483647]
        self.assertEqual(list(integer_wave(wave(32, values))[3]), values)

    def test_pcm16_callers_still_refuse_pcm24_specifically(self):
        with self.assertRaisesRegex(ValueError, 'requires 16 valid PCM bits'):
            pcm16_wave(wave(24, [0, 1], extensible=True))

    def test_valid_bits_mismatch_is_not_silently_renormalized(self):
        with self.assertRaisesRegex(ValueError, 'valid/container PCM bits differ'):
            integer_wave(wave(24, [0, 1], extensible=True, valid=20))

    def test_partial_pcm24_frame_and_riff_bounds_refuse_specifically(self):
        with self.assertRaisesRegex(ValueError, 'partial PCM reference frame'):
            integer_wave(wave(24, [], payload=b'\0'*5))
        raw = bytearray(wave(24, [0, 1]))
        raw[4:8] = struct.pack('<I', len(raw))
        with self.assertRaisesRegex(ValueError, 'RIFF size mismatch'):
            integer_wave(raw)

    def test_duplicate_format_or_data_chunks_are_not_accepted(self):
        for kind in ['fmt', 'data']:
            with self.subTest(kind=kind), self.assertRaises(ValueError):
                integer_wave(wave(24, [0, 1], duplicates=kind))

    def test_truncated_extensible_format_is_not_interpreted_as_pcm(self):
        raw = bytearray(wave(24, [0, 1], extensible=True))
        raw[36:38] = struct.pack('<H', 24)  # cbSize exceeds the stored format.
        with self.assertRaisesRegex(ValueError, 'unsupported extensible WAVE'):
            integer_wave(raw)


if __name__ == '__main__':
    unittest.main()
