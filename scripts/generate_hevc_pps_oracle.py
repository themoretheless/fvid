#!/usr/bin/env python3
"""Regenerate the synthetic PPS-update oracle; not used by ordinary tests."""
from pathlib import Path
import argparse
import subprocess
import tempfile

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--hm-decoder', type=Path, required=True)
args = parser.parse_args()

root = Path(__file__).resolve().parents[1]
fixtures = root / 'tests/fixtures/playback-errors'
mp4 = (fixtures / 'hevc-multislice-main.mp4').read_bytes()
index = mp4.index(b'hvcC')
size = int.from_bytes(mp4[index - 4:index], 'big')
config = mp4[index + 4:index + size - 4]
position = 23
annex_b = bytearray()
for _ in range(config[22]):
    count = int.from_bytes(config[position + 1:position + 3], 'big')
    position += 3
    for _ in range(count):
        size = int.from_bytes(config[position:position + 2], 'big')
        position += 2
        annex_b += b'\x00\x00\x00\x01' + config[position:position + size]
        position += size
packet = (fixtures / 'hevc-pps-update.packet').read_bytes()
position = 0
while position < len(packet):
    size = int.from_bytes(packet[position:position + 4], 'big')
    position += 4
    assert 0 < size <= len(packet) - position
    annex_b += b'\x00\x00\x00\x01' + packet[position:position + size]
    position += size
with tempfile.TemporaryDirectory(prefix='fvid-pps-oracle-') as temp:
    source = Path(temp) / 'update.hevc'
    output = Path(temp) / 'update.yuv'
    source.write_bytes(annex_b)
    subprocess.run([str(args.hm_decoder), '-b', str(source), '-o', str(output),
                    '--OutputBitDepth=8', '--OutputBitDepthC=8', '--SEIDecodedPictureHash=0'], check=True)
    actual = output.read_bytes()
    expected = (fixtures / 'hevc-multislice-main.yuv').read_bytes()[:128 * 128 * 3 // 2]
    assert actual == expected, 'PPS update changed the intra picture'
    (fixtures / 'hevc-pps-update.yuv').write_bytes(actual)
    print(f'Verified {len(actual)} YUV bytes')
