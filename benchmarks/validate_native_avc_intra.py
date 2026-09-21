#!/usr/bin/env python3
"""Pixel-by-pixel reference validation of FVid's CAVLC intra picture subset.
FFmpeg is used only here, as fixture encoder and reference decoder.
"""
import json
import re
from pathlib import Path
import subprocess
import tempfile
ROOT = Path(__file__).resolve().parents[1]

def run(*args, with_stderr=False):
    result = subprocess.run(args, cwd=ROOT, capture_output=True, text=True)
    if result.returncode:
        raise RuntimeError(f'{args}:\n{result.stderr}')
    return result.stderr if with_stderr else result.stdout


def main():
    log=run('cargo','build','--locked','--offline','--no-default-features','--example','decode_avc_intra','--message-format=json')
    artifacts=[json.loads(line) for line in log.splitlines() if line.startswith('{')]
    binary=next(a['executable'] for a in artifacts if a.get('reason')=='compiler-artifact' and a.get('target',{}).get('name')=='decode_avc_intra' and a.get('executable'))
    with tempfile.TemporaryDirectory(prefix='fvid-intra-pixels-') as tmp:
        for name,source,pixfmt,profile in [
            ('pattern','testsrc2=size=96x64:rate=3','yuv420p','baseline'),
            ('cropped','testsrc2=size=66x50:rate=3','yuv420p','baseline'),
            ('flat','color=c=gray:size=64x48:rate=3','yuv420p','baseline'),
            ('high10','testsrc2=size=96x64:rate=3','yuv420p10le','high10'),
            ('deblock','testsrc2=size=96x64:rate=3','yuv420p','baseline'),
            ('deblock10','testsrc2=size=96x64:rate=3','yuv420p10le','high10'),
            ('deblock_offsets','testsrc2=size=66x50:rate=3','yuv420p','baseline'),
            ('cabac','testsrc2=size=96x64:rate=3','yuv420p','main'),
            ('cabac10','testsrc2=size=96x64:rate=3','yuv420p10le','high10'),
            ('cabac_crop','testsrc2=size=66x50:rate=3','yuv420p','high'),
            ('cabac8','testsrc2=size=128x96:rate=3','yuv420p','high'),
            ('cabac8_10','testsrc2=size=128x96:rate=3','yuv420p10le','high10'),
            ('cabac8_mixed','testsrc2=size=128x96:rate=3','yuv420p','high'),
            ('cavlc8','testsrc2=size=128x96:rate=3','yuv420p','high'),
        ]:
            mp4=Path(tmp)/f'{name}.mp4';own=Path(tmp)/f'{name}.own.yuv';reference=Path(tmp)/f'{name}.ref.yuv'
            filtering = 'deblock=3,-2' if name == 'deblock_offsets' else 'deblock=0,0' if name.startswith('deblock') else 'deblock=0,0' if name.startswith('cabac') else 'no-deblock=1'
            eight = name in ('cabac8', 'cabac8_10', 'cabac8_mixed', 'cavlc8')
            encoder = run('ffmpeg','-v','info','-f','lavfi','-i',source,'-t','1','-pix_fmt',pixfmt,'-c:v','libx264',
                '-profile:v',profile,'-x264-params',f'cabac={int(name.startswith("cabac"))}:keyint=1:{filtering}:8x8dct={int(eight)}' + (':partitions=i8x8' if eight and name != 'cabac8_mixed' else ''),str(mp4),with_stderr=True)
            if eight:
                usage = re.search(r'8x8 transform intra:([0-9.]+)%', encoder)
                assert usage and float(usage[1]) > 0, encoder
            result=run(binary,str(mp4),str(own)).strip()
            run('ffmpeg','-v','error','-i',str(mp4),'-pix_fmt',pixfmt,'-f','rawvideo',str(reference))
            a=own.read_bytes();b=reference.read_bytes()
            assert len(a)==len(b),(name,len(a),len(b))
            mismatches=[(i,x,y) for i,(x,y) in enumerate(zip(a,b)) if x!=y]
            assert not mismatches,(name,len(mismatches),mismatches[:12])
            print(f'{name}: {result}; all {len(a)} YUV bytes match')

if __name__=='__main__':main()
