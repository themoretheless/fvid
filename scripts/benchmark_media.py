#!/usr/bin/env python3
"""End-to-end native media benchmark; decoded equality gates every variant."""
import argparse, datetime, hashlib, json, pathlib, random, statistics, subprocess, tempfile, time
import benchmark_media_reference as validation
ROOT = pathlib.Path(__file__).resolve().parents[1]

def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--binary', default=str(ROOT/'target/release/fvid'))
    parser.add_argument('--runs', type=int, default=5)
    args = parser.parse_args()
    if args.runs < 3: parser.error('at least 3 measured runs required')
    binary = pathlib.Path(args.binary).resolve()
    validation.BINARY = binary
    results = []
    rng = random.Random(20260905)
    with tempfile.TemporaryDirectory(prefix='fvid-media-bench-') as directory:
        work = pathlib.Path(directory)
        for width, height in [(1280,720),(1920,1080)]:
            source = work/'source.mp4'
            validation.ff('-f','lavfi','-i',f'testsrc2=size={width}x{height}:rate=30','-frames:v','30','-c:v','libx264','-preset','fast','-pix_fmt','yuv420p',source)
            for operation in ['remux','transcode','crop-vflip','crop-hflip-vflip','lossless-trim']:
                output = work/'output.mkv'
                crop = f'2:2:{width//2}:{height//2}'
                filters = f'crop={width//2}:{height//2}:2:2:exact=1,vflip' if operation=='crop-vflip' else None
                if operation=='crop-hflip-vflip': filters=f'crop={width//2}:{height//2}:2:2:exact=1,hflip,vflip'
                if operation=='lossless-trim': filters='trim=start=0.2:end=0.8,setpts=PTS-STARTPTS'
                native = [str(binary),'media','remux' if operation=='remux' else 'transcode-lossless',str(source),str(output)]
                if operation=='crop-vflip': native += ['--crop',crop,'--vflip']
                if operation=='crop-hflip-vflip': native += ['--crop',crop,'--hflip','--vflip']
                if operation=='lossless-trim': native += ['--from','0.2','--to','0.8']
                def reference(single):
                    command = ['ffmpeg','-nostdin','-v','error']
                    if single: command += ['-threads','1']
                    command += ['-i',str(source),'-map','0:v:0','-an']
                    if filters: command += ['-vf',filters]
                    command += ['-c:v','copy'] if operation=='remux' else ['-c:v','ffv1','-level','3','-pix_fmt','yuv420p']
                    if single: command += ['-threads','1','-filter_threads','1']
                    return command + [str(output)]
                commands = {'fvid':native,'ffmpeg_default':reference(False),'ffmpeg_1thread':reference(True)}
                expected = validation.pixels(source,filters)
                samples = {name:[] for name in commands}
                sizes = {}
                for name,command in commands.items():
                    validation.run(command)
                    assert validation.pixels(output)==expected, name
                    if operation=='remux': assert validation.hashes(output)==validation.hashes(source)
                    sizes[name]=output.stat().st_size
                    output.unlink()
                # Verification runs warm the OS cache and each executable once.
                for _ in range(args.runs):
                    order=list(commands);rng.shuffle(order)
                    for name in order:
                        start=time.perf_counter_ns();validation.run(commands[name]);elapsed=time.perf_counter_ns()-start
                        samples[name].append(elapsed/1e6);output.unlink()
                medians={name:statistics.median(times) for name,times in samples.items()}
                result=dict(width=width,height=height,frames=30,operation=operation,samples_ms=samples,median_ms=medians,output_bytes=sizes,decoded_equality=True,input_sha256=hashlib.sha256(source.read_bytes()).hexdigest(),commands=commands)
                results.append(result)
                print(width,operation,medians,flush=True)
            source.unlink()
    report=dict(created_utc=datetime.datetime.now(datetime.timezone.utc).isoformat(),runs=args.runs,seed=20260905,
                scope='Whole CLI wall time, cached input, local file output without fsync, 30 frames; no GPU. Same FFV1 level and pixel format; default and one-thread FFmpeg baselines. No claim of full FFmpeg parity or universal speedup.',
                binary_sha256=hashlib.sha256(binary.read_bytes()).hexdigest(),ffmpeg_version=validation.run(['ffmpeg','-version']).stdout.decode().splitlines()[0],
                source_sha256={str(p.relative_to(ROOT)):hashlib.sha256(p.read_bytes()).hexdigest() for p in [*ROOT.joinpath('crates/fvid-media/src').glob('*.rs'),ROOT/'src/media_cli.rs',pathlib.Path(__file__)]},results=results)
    (ROOT/'benchmarks/media-benchmark.json').write_text(json.dumps(report,indent=2)+'\n')

if __name__=='__main__':main()
