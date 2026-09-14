#!/usr/bin/env python3
"""Sample-equality-gated WAV trim timing."""
import datetime,hashlib,json,pathlib,random,statistics,tempfile,time
import validate_media as v
ROOT=pathlib.Path(__file__).resolve().parents[1];v.BINARY=ROOT/'target/release/fvid'
with tempfile.TemporaryDirectory(prefix='fvid-pcm-bench-') as temp:
    w=pathlib.Path(temp);source=w/'source.wav';output=w/'output.wav'
    v.ff('-f','lavfi','-i','sine=frequency=997:sample_rate=48000','-t','60','-ac','2','-c:a','pcm_s24le',source)
    commands={
        'fvid':[str(v.BINARY),'media','trim-pcm',str(source),str(output),'--from','40.125','--to','40.525'],
        'ffmpeg_seek':['ffmpeg','-nostdin','-v','error','-ss','40.125','-i',str(source),'-t','0.4','-c:a','pcm_s24le',str(output)],
        'ffmpeg_filter':['ffmpeg','-nostdin','-v','error','-i',str(source),'-af','atrim=start=40.125:end=40.525,asetpts=PTS-STARTPTS','-c:a','pcm_s24le',str(output)]}
    def samples(path):return v.ff('-i',path,'-c:a','pcm_s24le','-f','s24le','-').stdout
    expected=samples(source)[1926000*6:1945200*6]
    timings={name:[]for name in commands}
    for name,command in commands.items():v.run(command);assert samples(output)==expected,name;output.unlink()
    rng=random.Random(20260905)
    for _ in range(5):
        names=list(commands);rng.shuffle(names)
        for name in names:
            start=time.perf_counter_ns();v.run(commands[name]);timings[name].append((time.perf_counter_ns()-start)/1e6);output.unlink()
    report=dict(created_utc=datetime.datetime.now(datetime.timezone.utc).isoformat(),scope='60-second stereo 48kHz 24-bit WAV, retain 0.4 seconds. Whole CLI, cached input and file output without fsync. 5 randomized runs after equality warmup. Fvid scans the input; FFmpeg seek and filter baselines.',commands=commands,sample_equality=True,samples_ms=timings,median_ms={k:statistics.median(x)for k,x in timings.items()},binary_sha256=hashlib.sha256(v.BINARY.read_bytes()).hexdigest(),source_sha256={str(p.relative_to(ROOT)):hashlib.sha256(p.read_bytes()).hexdigest()for p in [*ROOT.joinpath('crates/fvid-media/src').glob('*.rs'),ROOT/'src/media_cli.rs',pathlib.Path(__file__)]})
    (ROOT/'benchmarks/pcm-benchmark.json').write_text(json.dumps(report,indent=2)+'\n');print(report['median_ms'])
