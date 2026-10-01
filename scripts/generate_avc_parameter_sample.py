#!/usr/bin/env python3
"""Generate synthetic AVC in-band parameter updates and independent YUV references."""
from pathlib import Path
import subprocess
import tempfile

fixtures = Path(__file__).resolve().parents[1] / 'tests/fixtures/playback-errors'
with tempfile.TemporaryDirectory(prefix='fvid-avc-parameters-') as temp:
    root = Path(temp)
    streams, references = [], []
    for index, size in enumerate(('64x64', '96x64')):
        stream, reference = root / f'{index}.h264', root / f'{index}.yuv'
        subprocess.run(['ffmpeg', '-v', 'error', '-f', 'lavfi', '-i',
            f'testsrc2=size={size}:rate=30:duration=0.1', '-c:v', 'libx264',
            '-profile:v', 'main', '-pix_fmt', 'yuv420p', '-bf', '0',
            '-x264-params', 'threads=1:keyint=30:repeat-headers=1',
            '-an', '-f', 'h264', str(stream)], check=True)
        subprocess.run(['ffmpeg', '-v', 'error', '-i', str(stream),
            '-pix_fmt', 'yuv420p', '-f', 'rawvideo', str(reference)], check=True)
        streams.append(stream.read_bytes())
        references.append(reference.read_bytes())
    joined = root / 'joined.h264'
    joined.write_bytes(b''.join(streams))
    subprocess.run(['ffmpeg', '-v', 'error', '-r', '30', '-i', str(joined),
        '-c', 'copy', '-tag:v', 'avc3', '-y', str(fixtures / 'avc-inband-resize.mp4')], check=True)
    assert len(references[0]) == 3 * 64 * 64 * 3 // 2
    assert len(references[1]) == 3 * 96 * 64 * 3 // 2
    (fixtures / 'avc-inband-resize.yuv').write_bytes(b''.join(references))
