#!/usr/bin/env python3
"""Native GUI source regression; FFmpeg is only fixture encoder/RGB oracle."""
import pathlib
import subprocess
import tempfile
ROOT = pathlib.Path(__file__).resolve().parents[1]
def run(args):
    result = subprocess.run(args, cwd=ROOT, capture_output=True, text=True)
    if result.returncode:
        raise RuntimeError(f'{args}\n{result.stderr}')
    return result
run(['cargo','build','--locked','--offline','--no-default-features','--example','decode_native_rgb','--example','native_camera_probe'])
with tempfile.TemporaryDirectory(prefix='fvid-playback-') as directory:
    directory = pathlib.Path(directory)
    for name, space, full, depth in [('601', 'smpte170m', False, 8), ('709', 'bt709', False, 8), ('full', 'bt709', True, 8), ('10bit', 'bt709', False, 10)]:
        for color in ['black', 'white', 'red', 'blue']:
            stem = directory / (name + '-' + color)
            video, native, oracle = [stem.with_suffix(ext) for ext in ['.mp4', '.native.rgb', '.oracle.rgb']]
            run(['ffmpeg','-v','error','-f','lavfi','-i',f'color=c={color}:s=32x32:r=25','-frames:v','3',
                 '-vf',f'scale=in_color_matrix=bt601:out_color_matrix={space}:out_range={"full" if full else "limited"}',
                 '-pix_fmt', 'yuv420p10le' if depth == 10 else 'yuv420p', '-c:v','libx264',
                 '-x264-params','cabac=0:bframes=0:weightp=0:keyint=30:scenecut=0',
                 '-colorspace', space, '-color_range', 'pc' if full else 'tv', str(video)])
            decoded = run([str(ROOT/'target/debug/examples/decode_native_rgb'),str(video),str(native)])
            assert decoded.stdout.strip() == 'frames=3'
            run(['ffmpeg','-v','error','-i',str(video),'-pix_fmt','rgb24','-f','rawvideo',str(oracle)])
            a,b = native.read_bytes(), oracle.read_bytes()
            assert len(a) == len(b) == 32*32*3*3
            maximum = max(abs(x-y) for x,y in zip(a,b))
            if maximum > 3: raise AssertionError(f'{name}/{color}: max RGB difference {maximum}')
            print(f'{name}/{color}: 3 frames, RGB delta <= {maximum}, rewind verified')

    for name, params, vfr in [
        ('camera-motion', 'cabac=0:bframes=0:weightp=0:keyint=30:scenecut=0', False),
        ('camera-b-motion', 'cabac=1:bframes=2:b-adapt=0:b-pyramid=normal:weightp=0:keyint=30:scenecut=0', False),
        ('camera-b-vfr', 'cabac=1:bframes=2:b-adapt=0:b-pyramid=normal:weightp=0:keyint=30:scenecut=0', True),
    ]:
        video = directory / (name+'.mp4')
        native = directory / (name+'.rgb')
        camera = directory / (name+'.bgra')
        run(['ffmpeg','-v','error','-f','lavfi','-i','testsrc2=s=96x64:r=25','-frames:v','12'] + (['-vf',r'setpts=(N+max(0\,N-6))/(25*TB)','-fps_mode','vfr'] if vfr else []) + [
             '-c:v','libx264','-qp','20','-x264-params',
             params,str(video)])
        run([str(ROOT/'target/debug/examples/decode_native_rgb'),str(video),str(native)])
        run([str(ROOT/'target/debug/examples/native_camera_probe'),str(video),str(camera)])
        rgb = native.read_bytes()
        expected = bytearray()
        for index in [0,1,8 if vfr else 10,0]:
            frame = rgb[index*96*64*3:(index+1)*96*64*3]
            for i in range(0,len(frame),3): expected.extend([frame[i+2],frame[i+1],frame[i],255])
        assert camera.read_bytes() == expected, 'MP4 camera frame selection/BGRA/rewind mismatch'
        print(name+': MP4 camera: exact selected BGRA frames at 0/40/400/0 ms, host timestamps verified')
