#!/usr/bin/env python3
"""Synthetic RExt intra-reference filtering oracles; generation only needs FFmpeg."""
from pathlib import Path
import re
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / 'tests/fixtures/playback-errors'


def unescape(data):
    return data.replace(b'\x00\x00\x03', b'\x00\x00')


def escape(data):
    result = bytearray()
    zeros = 0
    for value in data:
        if zeros == 2 and value <= 3:
            result.append(3)
            zeros = 0
        result.append(value)
        zeros = zeros + 1 if value == 0 else 0
    return bytes(result)


def rewrite(nal, disabled):
    kind = (nal[0] >> 1) & 63
    if kind not in (32, 33):
        return nal
    data = bytearray(unescape(nal[2:]))
    # VPS has 32 prefix bits before PTL; SPS has eight. No sublayers.
    offset = 4 if kind == 32 else 1
    data[offset] = (data[offset] & 0xe0) | 4  # format range extensions profile
    data[offset + 1:offset + 5] = bytes(4)  # no Main/Main10 compatibility claim
    if kind == 33:
        bits = ''.join(f'{value:08b}' for value in data)
        stop = bits.rfind('1')
        assert bits[stop - 1] == '0', 'source already has SPS extensions'
        # extension present, RExt only, and the nine range-extension flags.
        extension = '1' + '10000000' + '00000' + str(int(disabled)) + '000'
        bits = bits[:stop - 1] + extension + '1'
        bits += '0' * (-len(bits) % 8)
        data = bytes(int(bits[i:i + 8], 2) for i in range(0, len(bits), 8))
    return nal[:2] + escape(data)


def run(*args):
    subprocess.run(['ffmpeg', '-v', 'error', '-y', *map(str, args)], check=True)


with tempfile.TemporaryDirectory(prefix='fvid-hevc-smoothing-') as temporary:
    tmp = Path(temporary)
    for depth in (8, 10):
        pix = 'yuv420p' if depth == 8 else 'yuv420p10le'
        raw = tmp / 'source.hevc'
        run('-f', 'lavfi', '-i', 'testsrc2=size=64x64:rate=25', '-frames:v', 3,
            '-pix_fmt', pix, '-c:v', 'libx265', '-x265-params',
            'log-level=error:pools=none:frame-threads=1:ctu=32:wpp=0:aq-mode=0:qp=24:no-sao=1:no-deblock=1:keyint=1',
            '-f', 'hevc', raw)
        units = [nal for nal in re.split(b'\x00\x00\x00?\x01', raw.read_bytes()) if nal]
        references = []
        for disabled in (False, True):
            name = f'hevc-rext-smoothing-{depth}-' + ('disabled' if disabled else 'enabled')
            stream = tmp / 'updated.hevc'
            stream.write_bytes(b''.join(b'\x00\x00\x00\x01' + rewrite(nal, disabled) for nal in units))
            run('-r', 25, '-i', stream, '-c:v', 'copy', '-tag:v', 'hvc1', OUT / (name + '.mp4'))
            run('-i', stream, '-pix_fmt', pix, '-f', 'rawvideo', OUT / (name + '.yuv'))
            reference = (OUT / (name + '.yuv')).read_bytes()
            assert len(reference) == 3 * 64 * 64 * 3 // 2 * (1 if depth == 8 else 2)
            references.append(reference)
        assert references[0] != references[1], 'fixture does not exercise reference filtering'
