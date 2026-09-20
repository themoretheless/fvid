#!/usr/bin/env python3
"""Audio decode comparison preserving identical PCM precision."""
import argparse,datetime,hashlib,json,pathlib,random,statistics,tempfile,time
import validate_media as v
ROOT=pathlib.Path(__file__).resolve().parents[1];v.BINARY=ROOT/'target/release/fvid'
parser=argparse.ArgumentParser()
parser.add_argument('--baseline')
parser.add_argument('--binary',default=str(v.BINARY))
parser.add_argument('--runs',type=int,default=21)
parser.add_argument('--duration',type=int,default=30)
parser.add_argument('--min-delta',type=float,default=0.15)
args=parser.parse_args()
v.BINARY=pathlib.Path(args.binary).resolve()
if args.runs<3 or args.duration<1:parser.error('runs >=3, duration >=1 required')
results=[];rng=random.Random(20260905)
with tempfile.TemporaryDirectory(prefix='fvid-audio-bench-') as temp:
    w=pathlib.Path(temp)
    for encoder,extension,pcm,raw in [('aac','m4a','pcm_f32le','f32le'),('libmp3lame','mp3','pcm_f32le','f32le'),('flac','flac','pcm_s16le','s16le')]:
        source=w/f'source.{extension}';output=w/'out.wav'
        v.ff('-f','lavfi','-i','sine=frequency=997:sample_rate=48000','-t',str(args.duration),'-ac','2','-c:a',encoder,source)
        interval_from=args.duration//3;interval_to=min(args.duration,interval_from+5)
        for mode,fvid_flags,ffmpeg_filter in [
            ('full',[],[]),
            ('interval',['--from',str(interval_from),'--to',str(interval_to)],['-af',f'atrim=start={interval_from}:end={interval_to},asetpts=PTS-STARTPTS']),
        ]:
            commands={'fvid':[str(v.BINARY),'media','decode-audio',str(source),str(output),*fvid_flags],'ffmpeg':['ffmpeg','-nostdin','-v','error','-i',str(source),*ffmpeg_filter,'-c:a',pcm,str(output)]}
            if args.baseline:commands['fvid_before']=[str(pathlib.Path(args.baseline).resolve()),*commands['fvid'][1:]]
            expected=v.ff('-i',source,*ffmpeg_filter,'-c:a',pcm,'-f',raw,'-').stdout;times={k:[] for k in commands}
            for name,command in commands.items():v.run(command);assert v.ff('-i',output,'-c:a',pcm,'-f',raw,'-').stdout==expected;output.unlink()
            for _ in range(args.runs):
                names=list(commands);rng.shuffle(names)
                for name in names:
                    start=time.perf_counter_ns();v.run(commands[name]);times[name].append((time.perf_counter_ns()-start)/1e6);output.unlink()
            medians={k:statistics.median(x)for k,x in times.items()}
            item=dict(codec=encoder,mode=mode,pcm=pcm,sample_equality=True,commands=commands,samples_ms=times,median_ms=medians,delta_vs_ffmpeg=1-medians['fvid']/medians['ffmpeg']);results.append(item);print(encoder,mode,item['median_ms'],flush=True)
report=dict(created_utc=datetime.datetime.now(datetime.timezone.utc).isoformat(),scope=f'{args.duration}-second stereo 48kHz audio to WAV, full decode plus a 5-second middle interval, PCM precision matched; whole CLI cached-input/file-output without fsync, {args.runs} randomized runs after exact sample equality warmup.',baseline_sha256=hashlib.sha256(pathlib.Path(args.baseline).read_bytes()).hexdigest() if args.baseline else None,results=results,binary_sha256=hashlib.sha256(v.BINARY.read_bytes()).hexdigest(),source_sha256={str(p.relative_to(ROOT)):hashlib.sha256(p.read_bytes()).hexdigest()for p in [*ROOT.joinpath('crates/fvid-media/src').glob('*.rs'),ROOT/'src/media_cli.rs',pathlib.Path(__file__)]})
(ROOT/'benchmarks/audio-benchmark.json').write_text(json.dumps(report,indent=2)+'\n')
failures=[item for item in results if item['mode']=='interval' and item['delta_vs_ffmpeg']<=args.min_delta]
if failures:raise SystemExit('audio interval gate failed: '+', '.join(f"{item['codec']} {item['delta_vs_ffmpeg']:+.1%}" for item in failures))
