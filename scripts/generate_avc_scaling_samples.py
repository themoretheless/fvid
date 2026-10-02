#!/usr/bin/env python3
"""Synthetic scaling matrices, x264 CLI and independent JM YUV; no FFmpeg."""
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
with tempfile.TemporaryDirectory(prefix='fvid-avc-scaling-') as temp:
    directory = Path(temp)
    source = directory / 'source.yuv'
    raw = bytearray()
    for frame in range(8):
        for plane, side in enumerate([64, 32, 32]):
            raw.extend(24+(x*3+y*5+frame*17+plane*29)%112+((x//8+y//8)%2)*64
                       for y in range(side) for x in range(side))
    source.write_bytes(raw)
    custom = []
    for index, (name, count) in enumerate([('cqm4iy',16),('cqm4ic',16),('cqm4py',16),('cqm4pc',16),('cqm8i',64),('cqm8p',64)]):
        custom += ['--'+name, ','.join(str(7+index*2+(i*5+i//4)%23) for i in range(count))]
    for name, matrix in [('jvt',['--cqm','jvt']), ('custom', custom)]:
        mkv, stream, decoded = [directory / f'{name}.{ext}' for ext in ['mkv','264','yuv']]
        subprocess.run([args.x264,'--demuxer','raw','--input-csp','i420','--input-res','64x64','--fps','30',
            '--frames','8','--threads','1','--keyint','30','--bframes','2','--ref','3','--partitions','i8x8,p8x8,b8x8',
            '--profile','high','--muxer','mkv','-o',str(mkv),*matrix,str(source)],check=True)
        config, frames = read_mkv(mkv.read_bytes(),30)
        assert any(pts != index for index,(pts,_,_) in enumerate(frames)), 'source must retain B-frame reordering'
        stream.write_bytes(annexb(config,frames))
        subprocess.run([str(args.jm_decoder),'-d',str(args.jm_config),'-p',f'InputFile={stream}',
            '-p',f'OutputFile={decoded}','-p','FileFormat=0','-p','RefFile=nonexistent.yuv'],cwd=directory,check=True)
        reference = decoded.read_bytes()
        assert len(reference)==8*64*64*3//2
        (args.output/f'avc-scaling-{name}.mp4').write_bytes(mux(config,frames))
        (args.output/f'avc-scaling-{name}.yuv').write_bytes(reference)
