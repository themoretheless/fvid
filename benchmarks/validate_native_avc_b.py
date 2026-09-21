#!/usr/bin/env python3
"""Real B-picture decode-order integration; FFmpeg is fixture/oracle only."""
import pathlib
import subprocess
import tempfile
ROOT = pathlib.Path(__file__).resolve().parents[1]
def run(args):
    return subprocess.run(args, cwd=ROOT, check=True, capture_output=True, text=True)
run(['cargo','build','--locked','--offline','--no-default-features','--example','decode_avc_reordered','--example','decode_avc_ip'])
with tempfile.TemporaryDirectory(prefix='fvid-b-') as directory:
    tmp = pathlib.Path(directory)
    for name, params in [
        ('cabac-spatial', 'cabac=1:direct=spatial:weightb=0:b-pyramid=none'),
        ('cabac-temporal', 'cabac=1:direct=temporal:weightb=1:b-pyramid=none'),
        ('cavlc-temporal', 'cabac=0:direct=temporal:weightb=1:b-pyramid=none'),
        ('reference-b', 'cabac=1:direct=spatial:weightb=1:b-pyramid=normal'),
    ]:
        video, native, oracle = (tmp/(name+ext) for ext in ('.mp4','.native.yuv','.oracle.yuv'))
        run(['ffmpeg','-v','error','-f','lavfi','-i','testsrc2=s=64x64:r=25','-frames:v','24',
            '-c:v','libx264','-qp','24','-x264-params',
            'bframes=2:b-adapt=0:weightp=0:ref=3:keyint=30:scenecut=0:'+params,str(video)])
        types = run(['ffprobe','-v','error','-select_streams','v:0','-show_entries','frame=pict_type',
            '-of','csv=p=0',str(video)]).stdout
        if types.count('B') == 0: raise AssertionError('fixture has no B pictures')
        decoded = run([str(ROOT/'target/debug/examples/decode_avc_reordered'),str(video),str(native)])
        if decoded.stdout.strip() != 'frames=24': raise AssertionError(decoded.stdout)
        run(['ffmpeg','-v','error','-i',str(video),'-f','rawvideo','-pix_fmt','yuv420p',str(oracle)])
        if native.read_bytes() != oracle.read_bytes(): raise AssertionError(name+' pixels differ')
        streamed = tmp/(name+'.stream.yuv')
        result = run([str(ROOT/'target/debug/examples/decode_avc_ip'),str(video),str(streamed)])
        if result.stdout.strip() != 'frames=24' or streamed.read_bytes() != oracle.read_bytes():
            raise AssertionError(name+' streaming reorder/rewind differs')
        print(f'{name}: 24 frames, including {types.count("B")} B, byte-exact')
