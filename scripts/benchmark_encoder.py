#!/usr/bin/env python3
"""H264 lossless encode benchmark, source-pixel equality before timing."""
import datetime,hashlib,json,pathlib,random,statistics,tempfile,time
import validate_media as v
ROOT=pathlib.Path(__file__).resolve().parents[1];v.BINARY=ROOT/'target/release/fvid'
with tempfile.TemporaryDirectory(prefix='fvid-encoder-bench-') as temp:
    w=pathlib.Path(temp);source=w/'source.mp4';output=w/'output.mp4'
    v.ff('-f','lavfi','-i','testsrc2=size=1280x720:rate=30','-frames:v','60','-c:v','libx264','-preset','fast',source)
    commands={
        'fvid':[str(v.BINARY),'media','transcode',str(source),str(output),'--encoder','libx264','--encoder-option','crf=0','--encoder-option','preset=fast','--crop','2:2:640:360','--hflip'],
        'ffmpeg':['ffmpeg','-nostdin','-v','error','-i',str(source),'-an','-vf','crop=640:360:2:2:exact=1,hflip','-c:v','libx264','-crf','0','-preset','fast','-pix_fmt','yuv420p',str(output)]}
    expected=v.pixels(source,'crop=640:360:2:2:exact=1,hflip');sizes={};times={k:[]for k in commands}
    for name,command in commands.items():v.run(command);assert v.pixels(output)==expected,name;sizes[name]=output.stat().st_size;output.unlink()
    rng=random.Random(20260905)
    for _ in range(5):
        names=list(commands);rng.shuffle(names)
        for name in names:
            start=time.perf_counter_ns();v.run(commands[name]);times[name].append((time.perf_counter_ns()-start)/1e6);output.unlink()
    report=dict(created_utc=datetime.datetime.now(datetime.timezone.utc).isoformat(),scope='60 720p input frames, crop to 640x360 and hflip, libx264 CRF0 preset fast, MP4 output. Whole CLI cached-input/file-output without fsync, 5 randomized runs after pixel-equality warmup. No claim about lossy quality/performance or all encoders.',commands=commands,decoded_equality=True,samples_ms=times,median_ms={k:statistics.median(x)for k,x in times.items()},output_bytes=sizes,binary_sha256=hashlib.sha256(v.BINARY.read_bytes()).hexdigest(),source_sha256={str(p.relative_to(ROOT)):hashlib.sha256(p.read_bytes()).hexdigest()for p in [*ROOT.joinpath('crates/fvid-media/src').glob('*.rs'),ROOT/'src/media_cli.rs',pathlib.Path(__file__)]})
    (ROOT/'benchmarks/encoder-benchmark.json').write_text(json.dumps(report,indent=2)+'\n');print(report['median_ms'])
