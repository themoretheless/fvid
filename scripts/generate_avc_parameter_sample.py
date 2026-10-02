#!/usr/bin/env python3
"""Own synthetic avc3 resolution update: x264 CLI, owned muxer, JM YUV; no FFmpeg."""
import argparse
from pathlib import Path
import re
import subprocess
import tempfile
from avc_fixture_mp4 import read_mkv, mux, annexb, ints

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--x264', default='x264')
parser.add_argument('--jm-decoder', type=Path, required=True)
parser.add_argument('--jm-config', type=Path, required=True)
parser.add_argument('--output', type=Path, default=Path(__file__).resolve().parents[1] / 'tests/fixtures/playback-errors')
args = parser.parse_args()
args.output.mkdir(parents=True, exist_ok=True)
with tempfile.TemporaryDirectory(prefix='fvid-avc-parameters-') as temp:
    directory = Path(temp)
    initial = None
    joined, references = [], []
    for width in [64, 96]:
        source, mkv, stream, decoded = [directory / f'{width}-{ext}' for ext in ['source.yuv','video.mkv','stream.264','decoded.yuv']]
        raw = bytearray()
        for frame in range(3):
            for plane in range(3):
                w, h = (width,64) if plane == 0 else (width//2,32)
                raw.extend(24+(x*3+y*5+frame*17+plane*29)%112+((x//8+y//8)%2)*64
                           for y in range(h) for x in range(w))
        source.write_bytes(raw)
        subprocess.run([args.x264,'--demuxer','raw','--input-csp','i420','--input-res',f'{width}x64','--fps','30',
            '--frames','3','--threads','1','--keyint','30','--bframes','0','--profile','main',
            '--muxer','mkv','-o',str(mkv),str(source)],check=True)
        config, frames = read_mkv(mkv.read_bytes(),30)
        if initial is None:
            initial = config
        stream.write_bytes(annexb(config,frames))
        subprocess.run([str(args.jm_decoder),'-d',str(args.jm_config),'-p',f'InputFile={stream}',
            '-p',f'OutputFile={decoded}','-p','FileFormat=0','-p','RefFile=nonexistent.yuv'],cwd=directory,check=True)
        reference = decoded.read_bytes()
        assert len(reference)==3*width*64*3//2
        references.append(reference)
        # Put each sequence's own SPS/PPS in its IDR sample, not just avcC.
        parameters = b''.join(ints(len(nal))+nal for nal in re.split(b'\x00\x00\x00?\x01',annexb(config,[])) if nal)
        offset = len(joined)
        joined.extend((pts+offset, key, (parameters if i == 0 else b'')+packet)
                      for i,(pts,key,packet) in enumerate(frames))
    (args.output/'avc-inband-resize.mp4').write_bytes(mux(initial,joined,64,64,inband_parameters=True))
    (args.output/'avc-inband-resize.yuv').write_bytes(b''.join(references))
