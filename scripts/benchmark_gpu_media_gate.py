#!/usr/bin/env python3
"""Randomized, interleaved strict gate for native CUDA media fair pairs."""
import argparse,datetime,hashlib,json,pathlib,random,statistics,subprocess,tempfile,time
from common import ROOT,prepend_cuda_bin,release_binary

prepend_cuda_bin()
parser=argparse.ArgumentParser()
parser.add_argument('--binary',default=str(release_binary()))
parser.add_argument('--runs',type=int,default=21)
parser.add_argument('--min-delta',type=float,default=0.15)
parser.add_argument('--operations',help='comma-separated subset for tuning; does not overwrite the full report')
args=parser.parse_args()
binary=pathlib.Path(args.binary).resolve()
if args.runs<3:parser.error('runs >= 3 required')
validation_path=ROOT/'benchmarks/hw-validation.json'
if not validation_path.is_file():
    raise SystemExit('run scripts/validate_hw_cuda.py successfully before the GPU gate')
validation=json.loads(validation_path.read_text())
if validation.get('status')!='passed' or len(validation.get('checks',[]))<7:
    raise SystemExit('CUDA validation report is incomplete; rerun validate_hw_cuda.py')
if hashlib.sha256(binary.read_bytes()).hexdigest()!=validation.get('binary_sha256'):
    raise SystemExit('CUDA validation report does not match --binary; rerun validate_hw_cuda.py')

def run(command,stdout=subprocess.DEVNULL):
    result=subprocess.run([str(v)for v in command],stdout=stdout,stderr=subprocess.PIPE,timeout=180)
    if result.returncode:raise RuntimeError(result.stderr.decode(errors='replace'))
    return result.stdout

def fixture(path,seconds):
    run(['ffmpeg','-nostdin','-y','-v','error','-f','lavfi','-i',
         'testsrc2=size=1920x1080:rate=30','-t',seconds,'-pix_fmt','yuv420p',
         '-c:v','libx264','-preset','veryfast','-an',path])

with tempfile.TemporaryDirectory(prefix='fvid-gpu-gate-')as temp:
    w=pathlib.Path(temp)
    regular=w/'regular.mp4';decoded=w/'decode.mp4';fused=w/'fused.mp4';horizontal=w/'hflip.mp4';copy=w/'copy.mp4';crop_src=w/'crop.mp4';cut_src=w/'cut.mp4'
    fixture(regular,'10');fixture(decoded,'1');fixture(fused,'60');fixture(horizontal,'60');fixture(copy,'30');fixture(crop_src,'30');fixture(cut_src,'30')
    crop='240:134:960:540';vf_crop='crop=960:540:240:134'
    sources={'copy':copy,'crop':crop_src,'hflip':horizontal,'vflip':regular,
             'fused':fused,'cut':cut_src}
    flags={
      'copy':[],
      'crop':['--crop',crop],
      'hflip':['--hflip'],
      'vflip':['--vflip'],
      'fused':['--crop',crop,'--hflip','--vflip'],
      'cut':['--from','2','--to','22'],
    }
    filters={
      'copy':'',
      'crop':vf_crop,
      'hflip':'hflip',
      'vflip':'vflip',
      'fused':vf_crop+',hflip,vflip',
      'cut':'',
    }
    outputs={name:(w/f'{name}-f.mp4',w/f'{name}-x.mp4')for name in sources}
    pairs={}
    for name,source in sources.items():
        fvid=[binary,'media','hw-filter',source,outputs[name][0],*flags[name],'--quiet']
        ffmpeg=['ffmpeg','-nostdin','-y','-v','error','-hwaccel','cuda',
                '-hwaccel_output_format','cuda']
        if name=='cut':ffmpeg+=['-ss','2','-to','22']
        ffmpeg+=['-i',source]
        if filters[name]:
            ffmpeg+=['-vf','hwdownload,format=nv12,'+filters[name]+',hwupload_cuda']
        ffmpeg+=['-c:v','h264_nvenc','-preset','p1','-bf','0','-an',outputs[name][1]]
        pairs[name]=(fvid,ffmpeg)
    pairs['decode']=(
      [binary,'media','decode',decoded,'--device','0','--quiet'],
      ['ffmpeg','-nostdin','-v','error','-hwaccel','cuda',
       '-hwaccel_output_format','cuda','-i',decoded,'-an','-f','null','-'])
    if args.operations:
        requested=args.operations.split(',')
        unknown=[name for name in requested if name not in pairs]
        if unknown:parser.error('unknown operations: '+','.join(unknown))
        pairs={name:pairs[name] for name in requested}

    samples={name:{'fvid':[],'ffmpeg':[]}for name in pairs}
    rng=random.Random(20260917)
    def cleanup(name):
        for path in outputs.get(name,()):path.unlink(missing_ok=True)
    for name,(fvid,ffmpeg) in pairs.items():
        run(fvid);cleanup(name);run(ffmpeg);cleanup(name)
    for _ in range(args.runs):
        names=list(pairs);rng.shuffle(names)
        for name in names:
            variants=[('fvid',pairs[name][0]),('ffmpeg',pairs[name][1])]
            rng.shuffle(variants)
            for variant,command in variants:
                start=time.perf_counter_ns();run(command)
                samples[name][variant].append((time.perf_counter_ns()-start)/1e6)
                cleanup(name)
    results={};failures=[]
    for name,values in samples.items():
        medians={variant:statistics.median(items)for variant,items in values.items()}
        paired_deltas=[
            1-fvid/ffmpeg
            for fvid,ffmpeg in zip(values['fvid'],values['ffmpeg'],strict=True)
        ]
        delta=statistics.median(paired_deltas)
        results[name]=dict(
          median_ms=medians,
          delta_vs_ffmpeg=delta,
          median_paired_delta_vs_ffmpeg=delta,
          paired_deltas=paired_deltas,
          samples_ms=values)
        print(f'{name}: {medians["fvid"]:.3f} vs {medians["ffmpeg"]:.3f} ms, paired delta={delta:+.1%}')
        if delta<=args.min_delta:failures.append(f'{name} {delta:+.1%}')
    report=dict(
      created_utc=datetime.datetime.now(datetime.timezone.utc).isoformat(),
      scope=f'{args.runs} randomized rounds; pair order and operation order shuffled',
      minimum_delta=args.min_delta,
      validation_report='benchmarks/hw-validation.json',
      binary_sha256=hashlib.sha256(binary.read_bytes()).hexdigest(),
      results=results,status='failed' if failures else 'passed')
    if not args.operations:
        (ROOT/'benchmarks/gpu-media-gate.json').write_text(json.dumps(report,indent=2)+'\n')
    if failures:raise SystemExit('strict GPU media gate failed: '+', '.join(failures))
