#!/usr/bin/env python3
"""Compare raw decoded f32le with a PCM16 WAVE reference, without a delay search.

--lag is explicit: actual[reference_frame + lag] is compared with reference.
Metrics alone are not a codec conformance verdict. Select the appropriate
renderer, normative timing procedure and accuracy category independently.
"""
import argparse
import array
import hashlib
import json
import math
from pathlib import Path
import struct
import sys


def pcm16_wave(data):
    if len(data) < 12 or data[:4] != b'RIFF' or data[8:12] != b'WAVE':
        raise ValueError('expected RIFF/WAVE reference')
    end = 8 + struct.unpack_from('<I', data, 4)[0]
    if end != len(data):
        raise ValueError('reference RIFF size mismatch')
    fmt = payload = None
    at = 12
    while at < end:
        if at + 8 > end:
            raise ValueError('truncated WAVE chunk header')
        tag, size = struct.unpack_from('<4sI', data, at)
        start = at + 8
        if start + size + size % 2 > end:
            raise ValueError('WAVE chunk exceeds RIFF')
        if tag == b'fmt ':
            if fmt is not None:
                raise ValueError('duplicate WAVE format')
            fmt = data[start:start+size]
        if tag == b'data':
            if payload is not None:
                raise ValueError('multiple WAVE data chunks')
            payload = data[start:start+size]
        at = start + size + size % 2
    if fmt is None or len(fmt) < 16 or payload is None:
        raise ValueError('missing WAVE format or PCM')
    kind, channels, rate, byte_rate, stride, depth = struct.unpack_from('<HHIIHH', fmt)
    if kind == 0xfffe:
        guid = bytes.fromhex('0100000000001000800000aa00389b71')
        if len(fmt) < 40 or struct.unpack_from('<H', fmt, 16)[0] < 22 or fmt[24:40] != guid:
            raise ValueError('unsupported extensible WAVE reference')
        if struct.unpack_from('<H', fmt, 18)[0] != 16:
            raise ValueError('reference requires 16 valid PCM bits')
    elif kind != 1:
        raise ValueError('reference requires integer PCM')
    if not channels or not rate or depth != 16 or stride != channels*2 or byte_rate != rate*stride:
        raise ValueError('inconsistent PCM16 reference geometry')
    if len(payload) % stride:
        raise ValueError('partial PCM reference frame')
    samples = array.array('h', payload)
    if sys.byteorder != 'little':
        samples.byteswap()
    return channels, rate, samples


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('reference', type=Path)
    parser.add_argument('actual', type=Path)
    parser.add_argument('--lag', type=int, default=0)
    args = parser.parse_args()
    reference_bytes = args.reference.read_bytes()
    actual_bytes = args.actual.read_bytes()
    channels, rate, reference = pcm16_wave(reference_bytes)
    if len(actual_bytes) % (4*channels):
        raise ValueError('partial actual f32 sample frame')
    actual = array.array('f', actual_bytes)
    if sys.byteorder != 'little':
        actual.byteswap()
    if len(actual) != len(reference) or not all(math.isfinite(v) for v in actual):
        raise ValueError('actual length differs or contains non-finite samples')
    frames = len(reference)//channels
    start, end = max(0, -args.lag), min(frames, frames-args.lag)
    if start >= end:
        raise ValueError('empty comparison interval')
    output = []
    for ch in range(channels):
        error = energy = maximum = 0.0
        for i in range(start, end):
            value = reference[i*channels+ch]/32768
            delta = actual[(i+args.lag)*channels+ch]-value
            error += delta*delta
            energy += value*value
            maximum = max(maximum, abs(delta))
        output.append(dict(channel=ch, rmse=math.sqrt(error/(end-start)),
                           max_abs_error=maximum,
                           snr_db=10*math.log10(energy/error) if energy and error else None))
    print(json.dumps(dict(conformance_verdict=None, reference_sha256=hashlib.sha256(reference_bytes).hexdigest(),
                          actual_sha256=hashlib.sha256(actual_bytes).hexdigest(), rate=rate,
                          channels=channels, input_frames=frames, compared_frames=end-start,
                          explicit_lag=args.lag, per_channel=output), indent=2))


if __name__ == '__main__':
    main()
