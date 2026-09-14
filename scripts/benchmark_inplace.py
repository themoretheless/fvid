#!/usr/bin/env python3
"""Compare owned CPU in-place flips with a preserved binary and FFmpeg."""
import argparse, datetime, hashlib, json, pathlib, random, statistics, subprocess, tempfile, time
ROOT = pathlib.Path(__file__).resolve().parents[1]
p = argparse.ArgumentParser()
p.add_argument('--baseline', required=True)
p.add_argument('--runs', type=int, default=11)
a = p.parse_args()
assert a.runs >= 3
binary = ROOT / 'target/release/fvid'
rng = random.Random(20260905)
results = []
def run(cmd, **kwargs):
    return subprocess.run(cmd, check=True, stderr=subprocess.PIPE, **kwargs)
def digest(path):
    return hashlib.sha256(pathlib.Path(path).read_bytes()).hexdigest()
with tempfile.TemporaryDirectory(prefix='fvid-inplace-') as tmp:
    # Check all supported subsampling modes, including narrow and offset crops.
    for fmt in ['yuv420p', 'yuv422p', 'yuv444p']:
        src = ROOT / f'benchmarks/data/640x360-7-{fmt}.y4m'
        for crop in [None, (2, 2, 6, 4), (32, 16, 512, 288)]:
            for vertical in [False, True]:
                args = ['--hflip']; filters = []
                if crop:
                    x,y,w,h = crop
                    args += ['--crop', f'{x}:{y}:{w}:{h}']; filters += [f'crop={w}:{h}:{x}:{y}']
                filters += ['hflip']
                if vertical:
                    args += ['--vflip']; filters += ['vflip']
                output = pathlib.Path(tmp) / 'check.y4m'
                run([str(binary), str(src), str(output), *args])
                actual = run(['ffmpeg','-v','error','-i',str(output),'-f','rawvideo','-'], stdout=subprocess.PIPE).stdout
                expected = run(['ffmpeg','-v','error','-i',str(src),'-vf',','.join(filters),'-f','rawvideo','-'], stdout=subprocess.PIPE).stdout
                assert actual == expected, (fmt,crop,vertical)
                output.unlink()
    for w,h,n in [(1920,1080,120), (3840,2160,60)]:
        src = ROOT / f'benchmarks/data/{w}x{h}-{n}-yuv420p.y4m'
        for case in ['hflip', 'crop_hflip_vflip', 'copy']:
            args = []; filters = []
            if case == 'crop_hflip_vflip':
                x,y,cw,ch = w//8//2*2,h//8//2*2,w//2,h//2
                args += ['--crop',f'{x}:{y}:{cw}:{ch}']; filters += [f'crop={cw}:{ch}:{x}:{y}']
            if case != 'copy': args += ['--hflip']; filters += ['hflip']
            if case == 'crop_hflip_vflip': args += ['--vflip']; filters += ['vflip']
            ff = ['ffmpeg','-nostdin','-v','error','-filter_threads','1','-threads','1','-i',str(src)]
            if filters: ff += ['-vf',','.join(filters)]
            ff += ['-threads','1','-c:v','rawvideo','-pix_fmt','yuv420p','-f','yuv4mpegpipe','-']
            commands = {'before':[str(pathlib.Path(a.baseline).resolve()),str(src),'-',*args], 'after':[str(binary),str(src),'-',*args], 'ffmpeg_1thread':ff}
            samples = {k:[] for k in commands}
            for cmd in commands.values(): run(cmd, stdout=subprocess.DEVNULL)
            for _ in range(a.runs):
                order=list(commands); rng.shuffle(order)
                for name in order:
                    start=time.perf_counter_ns(); run(commands[name],stdout=subprocess.DEVNULL)
                    samples[name].append((time.perf_counter_ns()-start)/1e6)
            item=dict(resolution=f'{w}x{h}',frames=n,case=case,input_sha256=digest(src),commands=commands,samples_ms=samples,median_ms={k:statistics.median(v) for k,v in samples.items()})
            results.append(item); print(item['resolution'],case,item['median_ms'],flush=True)
report=dict(created_utc=datetime.datetime.now(datetime.timezone.utc).isoformat(),correctness_cases=18,method=f'CPU Y4M; warm cached file input to null sink; includes startup and serialization; 1 warmup, {a.runs} seeded interleaved rounds; FFmpeg single-thread; no codec/GPU or disk durability claims.',ffmpeg=run(['ffmpeg','-version'],stdout=subprocess.PIPE).stdout.decode().splitlines()[0],baseline_sha256=digest(a.baseline),binary_sha256=digest(binary),source_sha256={str(p.relative_to(ROOT)):digest(p) for p in [ROOT/'src/lib.rs',ROOT/'src/backend.rs',pathlib.Path(__file__)]},results=results)
(ROOT/'benchmarks/inplace-benchmark.json').write_text(json.dumps(report,indent=2)+'\n')
