#!/usr/bin/env python3
"""Synthetic HEVC RExt smoothing/rotation fixtures, generated without FFmpeg."""
import argparse
from pathlib import Path
import subprocess
import tempfile
import re
from hevc_fixture_mp4 import mux

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


def rewrite(nal, disabled, rotation=False):
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
        flags = (str(int(disabled)) + '00000000') if rotation else ('00000' + str(int(disabled)) + '000')
        extension = '1' + '10000000' + flags
        bits = bits[:stop - 1] + extension + '1'
        bits += '0' * (-len(bits) % 8)
        data = bytes(int(bits[i:i + 8], 2) for i in range(0, len(bits), 8))
    return nal[:2] + escape(data)



parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--rotation', action='store_true')
parser.add_argument('--hm-encoder', type=Path, required=True)
parser.add_argument('--hm-decoder', type=Path, required=True)
parser.add_argument('--hm-config', type=Path, required=True)
parser.add_argument('--output', type=Path, default=Path(__file__).resolve().parents[1] / 'tests/fixtures/playback-errors')
args = parser.parse_args()
args.output.mkdir(parents=True, exist_ok=True)
with tempfile.TemporaryDirectory(prefix='fvid-hevc-smoothing-') as directory:
    tmp = Path(directory)
    for depth in (8, 10):
        source = tmp / 'source.yuv'
        raw = bytearray()
        for frame in range(3):
            for plane, side in enumerate((64, 32, 32)):
                for y in range(side):
                    for x in range(side):
                        value = (24 + plane * 19 + (x * 3 + y * 5 + frame * 17) % 112 + ((x // 8 + y // 8) % 2) * 64) << (depth - 8)
                        raw.extend(bytes([value]) if depth == 8 else value.to_bytes(2, 'little'))
        source.write_bytes(raw)
        for mode in (('skip', 'bypass') if args.rotation else ('smoothing',)):
            references = []
            results = []
            for switched in (False, True):
                name = (f'hevc-rext-rotation-{depth}-{mode}-' + ('enabled' if switched else 'disabled')) if args.rotation else (f'hevc-rext-smoothing-{depth}-' + ('disabled' if switched else 'enabled'))
                stream, recon, decoded = tmp / 'stream.hevc', tmp / 'recon.yuv', tmp / 'decoded.yuv'
                subprocess.run([str(args.hm_encoder), '-c', str(args.hm_config), '-i', str(source), '-b', str(stream), '-o', str(recon),
                    '-wdt', '64', '-hgt', '64', '-fr', '25', '-f', '3', f'--InputBitDepth={depth}', f'--InternalBitDepth={depth}',
                    '--InputChromaFormat=420', '--MaxCUWidth=32', '--MaxCUHeight=32', '--MaxPartitionDepth=3',
                    f'--QuadtreeTULog2MaxSize={2 if args.rotation else 4}', f'--TransformSkip={int(args.rotation)}',
                    '--ResidualRotation=0', f'--IntraReferenceSmoothing={int(not switched or args.rotation)}',
                    '--ImplicitResidualDPCM=0', '--ExplicitResidualDPCM=0', '--GolombRiceParameterAdaptation=0',
                    '--SingleSignificanceMapContext=0', '--HighPrecisionPredictionWeighting=0', '--CrossComponentPrediction=0',
                    '--SAO=0', '--LoopFilterDisable=1', '--QP=24', f'--TransquantBypassEnable={int(mode == "bypass")}',
                    f'--CUTransquantBypassFlagForce={int(mode == "bypass")}'], check=True)
                if args.rotation and switched:
                    units = [nal for nal in re.split(b'\x00\x00\x00?\x01', stream.read_bytes()) if nal]
                    stream.write_bytes(b''.join(b'\x00\x00\x00\x01' + rewrite(nal, True, True) for nal in units))
                subprocess.run([str(args.hm_decoder), '-b', str(stream), '-o', str(decoded), f'--OutputBitDepth={depth}', f'--OutputBitDepthC={depth}'], check=True)
                reference = decoded.read_bytes()
                if not (args.rotation and switched):
                    assert reference == recon.read_bytes(), 'HM reconstruction and decoding differ'
                assert len(reference) == 3 * 64 * 64 * 3 // 2 * (1 if depth == 8 else 2)
                references.append(reference)
                results.append((name, mux(stream.read_bytes(), depth), reference))
            assert references[0] != references[1], 'source does not exercise selected tool'
            for name, mp4, reference in results:
                (args.output / (name + '.mp4')).write_bytes(mp4)
                (args.output / (name + '.yuv')).write_bytes(reference)
