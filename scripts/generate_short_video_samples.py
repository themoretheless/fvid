#!/usr/bin/env python3
"""Own one-second YUV patterns; standalone x264/JM and libvpx CLI, no FFmpeg."""
import argparse
from pathlib import Path
import subprocess
import tempfile
from avc_fixture_mp4 import read_mkv, mux, annexb

parser=argparse.ArgumentParser(description=__doc__)
parser.add_argument('--x264',default='x264')
parser.add_argument('--jm-decoder',type=Path,required=True)
parser.add_argument('--jm-config',type=Path,required=True)
parser.add_argument('--vpx-encoder',type=Path,required=True)
parser.add_argument('--vpx-decoder',type=Path,required=True)
parser.add_argument('--output',type=Path,default=Path(__file__).resolve().parents[1]/'tests/fixtures/short')
args=parser.parse_args()
args.output.mkdir(parents=True,exist_ok=True)
with tempfile.TemporaryDirectory(prefix='fvid-short-clips-') as temp:
    directory=Path(temp)
    source=directory/'source.yuv'
    raw=bytearray()
    for frame in range(12):
        for plane in range(3):
            width,height=(96,64) if plane==0 else (48,32)
            raw.extend(24+(x*3+y*5+frame*17+plane*29)%112+((x//8+y//8)%2)*64
                       for y in range(height) for x in range(width))
    source.write_bytes(raw)
    for name,profile,bframes in [('avc-baseline','baseline',0),('avc-bframes','high',2)]:
        mkv,stream,decoded=[directory/(name+ext) for ext in ['.mkv','.264','.yuv']]
        subprocess.run([args.x264,'--demuxer','raw','--input-csp','i420','--input-res','96x64','--fps','12',
            '--frames','12','--threads','1','--keyint','4','--min-keyint','4','--scenecut','0',
            '--bframes',str(bframes),'--b-adapt','0','--profile',profile,'--muxer','mkv','-o',str(mkv),str(source)],check=True)
        config,frames=read_mkv(mkv.read_bytes(),12)
        if bframes:
            assert any(pts!=i for i,(pts,_,_) in enumerate(frames)), 'B-frame reordering required'
        assert sum(key for _,key,_ in frames)==3, 'three fixed GOPs required'
        stream.write_bytes(annexb(config,frames))
        subprocess.run([str(args.jm_decoder.resolve()),'-d',str(args.jm_config.resolve()),'-p','InputFile='+str(stream),
            '-p','OutputFile='+str(decoded),'-p','FileFormat=0','-p','RefFile=nonexistent.yuv'],cwd=directory,check=True)
        reference=decoded.read_bytes()
        assert len(reference)==len(raw)
        (args.output/(name+'.mp4')).write_bytes(mux(config,frames,96,64,12))
        (args.output/(name+'.yuv')).write_bytes(reference)
    video=directory/'vp9-motion.webm'
    decoded=directory/'vp9-motion.yuv'
    subprocess.run([str(args.vpx_encoder.resolve()),'--codec=vp9','--debug','--webm','--width=96','--height=64','--fps=12/1',
        '--limit=12','--threads=1','--passes=1','--lag-in-frames=0','--auto-alt-ref=0','--kf-min-dist=4','--kf-max-dist=4',
        '--end-usage=q','--cq-level=33','--cpu-used=2','--output='+str(video),str(source)],check=True)
    subprocess.run([str(args.vpx_decoder.resolve()),'--codec=vp9','--i420','--rawvideo','--output='+str(decoded),str(video)],check=True)
    reference=decoded.read_bytes()
    assert len(reference)==len(raw)
    (args.output/video.name).write_bytes(video.read_bytes())
    (args.output/decoded.name).write_bytes(reference)
