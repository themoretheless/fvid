#!/usr/bin/env python3
"""Seek qualification and timing against full decoding and FFmpeg input seeking."""
import datetime,hashlib,json,pathlib,random,statistics,tempfile,time
import validate_media as v
ROOT=pathlib.Path(__file__).resolve().parents[1]
v.BINARY=ROOT/'target/release/fvid'
with tempfile.TemporaryDirectory(prefix='fvid-seek-bench-') as temp:
    w=pathlib.Path(temp);src=w/'source.mp4';out=w/'output.mkv'
    v.ff('-f','lavfi','-i','testsrc2=size=1280x720:rate=25','-t','10','-c:v','libx264','-preset','fast','-bf','3','-g','25','-keyint_min','25','-sc_threshold','0',src)
    native=[str(v.BINARY),'media','transcode-lossless',str(src),str(out),'--from','8.12','--to','8.52']
    ff=['ffmpeg','-nostdin','-v','error','-ss','8.12','-i',str(src),'-t','0.4','-an','-c:v','ffv1','-level','3','-pix_fmt','yuv420p',str(out)]
    commands={'fvid_full_decode':native,'fvid_seek':native+['--seek'],'ffmpeg_seek':ff}
    expected=v.pixels(src,'trim=start=8.12:end=8.52');stats={};samples={k:[] for k in commands}
    for name,command in commands.items():
        result=v.run(command)
        assert v.pixels(out)==expected,name
        if name.startswith('fvid'):stats[name]=json.loads(result.stdout)
        out.unlink()
    rng=random.Random(20260905)
    for _ in range(5):
        order=list(commands);rng.shuffle(order)
        for name in order:
            start=time.perf_counter_ns();v.run(commands[name]);samples[name].append((time.perf_counter_ns()-start)/1e6);out.unlink()
    report=dict(created_utc=datetime.datetime.now(datetime.timezone.utc).isoformat(),scope='720p H264 closed GOP with B-frames, 250 input frames, select 10 frames near end; whole CLI wall time, file output without fsync, warm cache, 5 randomized runs. Fvid seek still decodes the remainder to EOF.',commands=commands,samples_ms=samples,median_ms={k:statistics.median(x)for k,x in samples.items()},stats=stats,decoded_equality=True,input_sha256=hashlib.sha256(src.read_bytes()).hexdigest(),binary_sha256=hashlib.sha256(v.BINARY.read_bytes()).hexdigest(),source_sha256={str(p.relative_to(ROOT)):hashlib.sha256(p.read_bytes()).hexdigest()for p in [*ROOT.joinpath('crates/fvid-media/src').glob('*.rs'),ROOT/'src/media_cli.rs',pathlib.Path(__file__)]})
    (ROOT/'benchmarks/seek-benchmark.json').write_text(json.dumps(report,indent=2)+'\n')
    print(json.dumps(report['median_ms'],indent=2))
    print(json.dumps(stats,indent=2))
