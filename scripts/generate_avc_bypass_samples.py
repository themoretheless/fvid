#!/usr/bin/env python3
"""Own integer lossless AVC patterns, x264 CLI and independent JM YUV; no FFmpeg."""
import argparse
from pathlib import Path
import subprocess
import tempfile
from avc_fixture_mp4 import read_mkv, mux, annexb

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--x264', default='x264')
parser.add_argument('--jm-decoder', type=Path, required=True)
parser.add_argument('--jm-config', type=Path, required=True)
parser.add_argument('--output', type=Path, default=Path(__file__).resolve().parents[1] / 'tests/fixtures/playback-errors')
args = parser.parse_args()
args.output.mkdir(parents=True, exist_ok=True)
with tempfile.TemporaryDirectory(prefix='fvid-avc-bypass-') as temp:
    directory = Path(temp)
    for name, cabac, depth in [('lossless', True, 8), ('cavlc', False, 8), ('main10', True, 10), ('cavlc10', False, 10)]:
        original, mkv, stream, decoded = [directory / f'{name}-{ext}' for ext in ['original.yuv','video.mkv','stream.264','decoded.yuv']]
        raw = bytearray()
        for frame in range(8):
            for plane, side in enumerate([64, 32, 32]):
                for y in range(side):
                    for x in range(side):
                        value = 24+(x*3+y*5+frame*17+plane*29)%112+((x//8+y//8)%2)*64
                        if depth == 10:
                            value = (value << 2) + (x+y+frame+plane)%4
                        raw.extend(bytes([value]) if depth == 8 else value.to_bytes(2,'little'))
        original.write_bytes(raw)
        subprocess.run([args.x264, '--demuxer','raw','--input-csp','i420','--input-res','64x64','--fps','30',
            '--input-depth',str(depth),'--output-depth',str(depth),'--frames','8','--threads','1','--keyint','30',
            '--bframes','2','--ref','3','--qp','0','--muxer','mkv','-o',str(mkv),
            *([] if cabac else ['--no-cabac']), str(original)],check=True)
        config, frames = read_mkv(mkv.read_bytes(),30)
        stream.write_bytes(annexb(config,frames))
        subprocess.run([str(args.jm_decoder),'-d',str(args.jm_config),'-p',f'InputFile={stream}',
            '-p',f'OutputFile={decoded}','-p','FileFormat=0','-p','RefFile=nonexistent.yuv'],cwd=directory,check=True)
        reference = decoded.read_bytes()
        assert reference == raw, f'{name}: independent JM decode is not lossless'
        (args.output/f'avc-bypass-{name}.mp4').write_bytes(mux(config,frames))
        (args.output/f'avc-bypass-{name}.yuv').write_bytes(reference)
