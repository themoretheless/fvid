#!/usr/bin/env python3
"""Initial I/P integration oracle. FFmpeg is only the fixture encoder/oracle."""
import pathlib
import subprocess
import tempfile
ROOT = pathlib.Path(__file__).resolve().parents[1]
def run(args):
    return subprocess.run(args, cwd=ROOT, check=True, capture_output=True, text=True)
run(['cargo','build','--locked','--offline','--no-default-features','--example','decode_avc_ip','--example','mp4_packets'])
with tempfile.TemporaryDirectory(prefix='fvid-ip-') as tmp:
    tmp = pathlib.Path(tmp)
    for color in ('gray', 'red', 'blue'):
        video, native, oracle = (tmp / (color + ext) for ext in ('.mp4','.native.yuv','.oracle.yuv'))
        run(['ffmpeg','-v','error','-f','lavfi','-i',f'color=c={color}:s=32x32:r=25','-frames:v','5',
             '-c:v','libx264','-profile:v','baseline','-x264-params',
             'cabac=0:bframes=0:weightp=0:keyint=30:scenecut=0',str(video)])
        types = run(['ffprobe','-v','error','-select_streams','v:0','-show_entries','frame=pict_type',
                     '-of','csv=p=0',str(video)]).stdout
        if types.count('P') != 4 or types.count('I') != 1:
            raise AssertionError(f'unexpected frame types: {types}')
        decoded = run([str(ROOT/'target/debug/examples/decode_avc_ip'),str(video),str(native)])
        if decoded.stdout.strip() != 'frames=5':
            raise AssertionError(decoded.stdout)
        run(['ffmpeg','-v','error','-i',str(video),'-f','rawvideo','-pix_fmt','yuv420p',str(oracle)])
        a,b = native.read_bytes(),oracle.read_bytes()
        if a != b:
            raise AssertionError(f'{color}: decoded YUV differs')
        print(f'{color}: 1 I + 4 P frames, all {len(a)} bytes match')

    raw=tmp/'ramp.yuv'
    raw.write_bytes(b''.join(bytes([64+i*2])*1024+bytes([128])*512 for i in range(8)))
    video,native,oracle=(tmp/('ramp'+ext) for ext in ('.mp4','.native.yuv','.oracle.yuv'))
    run(['ffmpeg','-v','error','-f','rawvideo','-pixel_format','yuv420p','-video_size','32x32','-framerate','25',
         '-i',str(raw),'-c:v','libx264','-qp','20','-profile:v','baseline','-x264-params',
         'cabac=0:bframes=0:weightp=0:keyint=30:scenecut=0:ipratio=1',str(video)])
    run([str(ROOT/'target/debug/examples/decode_avc_ip'),str(video),str(native)])
    run(['ffmpeg','-v','error','-i',str(video),'-f','rawvideo','-pix_fmt','yuv420p',str(oracle)])
    if native.read_bytes()!=oracle.read_bytes():
        raise AssertionError('mixed intra/inter ramp pixels differ')
    print('ramp: 8 mixed intra/inter frames match')
    for name, source, profile, params, frames in [
        ('motion', ['-f','lavfi','-i','testsrc2=s=96x64:r=25','-frames:v','12'], 'baseline', 'weightp=0',12),
        ('cabac-motion', ['-f','lavfi','-i','testsrc2=s=96x64:r=25','-frames:v','24'], 'main', 'cabac=1:weightp=0',24),
        ('cabac-high8', ['-f','lavfi','-i','testsrc2=s=128x96:r=25','-frames:v','24'], 'high', 'cabac=1:weightp=0:8x8dct=1:analyse=all',24),
        ('cabac-high10', ['-f','lavfi','-i','testsrc2=s=96x64:r=25','-frames:v','12','-pix_fmt','yuv420p10le'], 'high10', 'cabac=1:weightp=0',12),
        ('high8-motion', ['-f','lavfi','-i','testsrc2=s=128x96:r=25','-frames:v','24'], 'high', 'weightp=0:8x8dct=1:analyse=all',24),
        ('constrained', ['-f','lavfi','-i','testsrc2=s=96x64:r=25','-frames:v','12'], 'baseline', 'weightp=0:constrained-intra=1',12),
        ('weighted', ['-f','rawvideo','-pixel_format','yuv420p','-video_size','32x32','-framerate','25','-i',str(raw)], 'main', 'weightp=2:ipratio=1',8),
    ]:
        video,native,oracle=(tmp/(name+ext) for ext in ('.mp4','.native.yuv','.oracle.yuv'))
        run(['ffmpeg','-v','error']+source+['-c:v','libx264','-qp','20','-profile:v',profile,
             '-x264-params','cabac=0:bframes=0:keyint=30:scenecut=0:'+params,str(video)])
        if name=='high8-motion':
            inspection=run([str(ROOT/'target/debug/examples/mp4_packets'),str(video)]).stderr
            blocks=sum(int(line.split(',')[1]) for line in inspection.splitlines() if line.startswith('AVC_INTER_8X8,'))
            if blocks == 0: raise AssertionError('high profile fixture encoded no inter 8x8 transforms')
            print(f'high8-motion: {blocks} inter 8x8-transform macroblocks inspected')
        if name=='constrained':
            trace=run(['ffmpeg','-i',str(video),'-c','copy','-bsf:v','trace_headers','-f','null','-']).stderr
            if not any('constrained_intra_pred_flag' in line and line.rstrip().endswith('= 1') for line in trace.splitlines()):
                raise AssertionError('constrained fixture did not enable constrained intra prediction')
        if name=='weighted':
            trace=run(['ffmpeg','-i',str(video),'-c','copy','-bsf:v','trace_headers','-f','null','-']).stderr
            if not any('luma_weight_l0_flag' in line and line.rstrip().endswith('= 1') for line in trace.splitlines()):
                raise AssertionError('weighted fixture did not encode explicit luma weights')
        result=run([str(ROOT/'target/debug/examples/decode_avc_ip'),str(video),str(native)])
        if result.stdout.strip()!=f'frames={frames}': raise AssertionError(result.stdout)
        run(['ffmpeg','-v','error','-i',str(video),'-f','rawvideo','-pix_fmt','yuv420p10le' if name=='cabac-high10' else 'yuv420p',str(oracle)])
        if native.read_bytes()!=oracle.read_bytes(): raise AssertionError(name+' pixels differ')
        print(f'{name}: {frames} frames match')
