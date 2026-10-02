#!/usr/bin/env python3
"""Own synthetic two-slice AVC patterns; x264 CLI and independent JM, no FFmpeg."""
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
with tempfile.TemporaryDirectory(prefix='fvid-avc-multislice-') as temp:
    directory = Path(temp)
    for name, cabac, depth in [('ipb', True, 8), ('cavlc', False, 8), ('cabac10', True, 10), ('cavlc10', False, 10)]:
        source, mkv, stream, decoded = [directory / f'{name}-{ext}' for ext in ['source.yuv','video.mkv','stream.264','decoded.yuv']]
        raw = bytearray()
        for frame in range(8):
            for plane in range(3):
                width, height = (128,96) if plane == 0 else (64,48)
                for y in range(height):
                    for x in range(width):
                        value = 24+(x*3+y*5+frame*17+plane*29)%112+((x//8+y//8)%2)*64
                        if depth == 10:
                            value = (value << 2)+(x+y+frame+plane)%4
                        raw.extend(bytes([value]) if depth == 8 else value.to_bytes(2,'little'))
        source.write_bytes(raw)
        subprocess.run([args.x264,'--demuxer','raw','--input-csp','i420','--input-res','128x96','--fps','30',
            '--input-depth',str(depth),'--output-depth',str(depth),'--frames','8','--threads','1','--slices','2',
            '--keyint','30','--bframes','2','--b-adapt','0','--ref','3','--crf','18','--muxer','mkv','-o',str(mkv),
            *([] if cabac else ['--no-cabac']),str(source)],check=True)
        config, frames = read_mkv(mkv.read_bytes(),30)
        assert any(pts != i for i,(pts,_,_) in enumerate(frames)), 'fixture must retain B-frame reordering'
        stream.write_bytes(annexb(config,frames))
        subprocess.run([str(args.jm_decoder),'-d',str(args.jm_config),'-p',f'InputFile={stream}',
            '-p',f'OutputFile={decoded}','-p','FileFormat=0','-p','RefFile=nonexistent.yuv'],cwd=directory,check=True)
        reference = decoded.read_bytes()
        assert len(reference)==8*128*96*3//2*(1 if depth == 8 else 2)
        (args.output/f'avc-multislice-{name}.mp4').write_bytes(mux(config,frames,128,96))
        (args.output/f'avc-multislice-{name}.yuv').write_bytes(reference)
