#!/usr/bin/env python3
"""Randomized, interleaved strict gate for all native CPU media fair pairs."""
import argparse,datetime,hashlib,json,pathlib,random,statistics,subprocess,tempfile,time
from common import ROOT,release_binary

parser=argparse.ArgumentParser()
parser.add_argument('--binary',default=str(release_binary()))
parser.add_argument('--runs',type=int,default=21)
parser.add_argument('--min-delta',type=float,default=0.15)
args=parser.parse_args()
binary=pathlib.Path(args.binary).resolve()
if args.runs<3:parser.error('runs >= 3 required')

def run(command,stdout=subprocess.DEVNULL):
    result=subprocess.run([str(v)for v in command],stdout=stdout,stderr=subprocess.PIPE,timeout=300)
    if result.returncode:raise RuntimeError(result.stderr.decode(errors='replace'))
    return result.stdout

def fixture(path,seconds,gop=False):
    command=['ffmpeg','-nostdin','-y','-v','error','-f','lavfi','-i','testsrc2=size=1920x1080:rate=30','-t',seconds,'-pix_fmt','yuv420p','-c:v','libx264','-preset','veryfast']
    if gop:command+=['-bf','0','-g','30','-keyint_min','30','-sc_threshold','0']
    run(command+['-an',path])

validation=json.loads((ROOT/'benchmarks/media-validation.json').read_text())
if validation.get('status')!='passed' or len(validation.get('checks',[]))<524:
    raise SystemExit('run scripts/validate_media.py successfully before the CPU gate')
if hashlib.sha256(binary.read_bytes()).hexdigest()!=validation.get('binary_sha256'):
    raise SystemExit('validation report does not match --binary; rerun validate_media.py')

with tempfile.TemporaryDirectory(prefix='fvid-cpu-gate-')as temp:
    w=pathlib.Path(temp);short=w/'short.mp4';brief=w/'brief.mp4';decode=w/'decode.mp4';medium=w/'medium.mp4';gop=w/'gop.mp4'
    fixture(short,'2');fixture(brief,'1');fixture(decode,'4');fixture(medium,'5');fixture(gop,'10',True)
    bgop=w/'bgop.mp4'
    run(['ffmpeg','-nostdin','-y','-v','error','-f','lavfi','-i','testsrc2=size=1920x1080:rate=30','-t','6','-pix_fmt','yuv420p','-c:v','libx264','-preset','veryfast','-bf','3','-g','30','-keyint_min','30','-sc_threshold','0','-an',bgop])
    hgop=w/'hgop.mp4'
    # HEVC+AAC remux: video-only copy sits on process-start noise; A/V at modest res lifts headroom.
    run(['ffmpeg','-nostdin','-y','-v','error','-f','lavfi','-i','testsrc2=size=640x360:rate=30','-f','lavfi','-i','sine=frequency=440:sample_rate=48000','-t','48','-pix_fmt','yuv420p','-c:v','libx265','-preset','ultrafast','-x265-params','keyint=30:min-keyint=30:scenecut=0:bframes=3:b-adapt=0:open-gop=0:log-level=error','-c:a','aac','-b:a','128k',hgop])
    # Few-frame filter fixtures: long 1080p clips bury wrapper delta under shared libavfilter work.
    rotate_src=w/'rotate.mp4';filter_src=w/'filter.mp4'
    run(['ffmpeg','-nostdin','-y','-v','error','-f','lavfi','-i','testsrc2=size=1920x1080:rate=30','-frames:v','3','-pix_fmt','yuv420p','-c:v','libx264','-preset','veryfast','-an',rotate_src])
    run(['ffmpeg','-nostdin','-y','-v','error','-f','lavfi','-i','testsrc2=size=1920x1080:rate=30','-frames:v','5','-pix_fmt','yuv420p','-c:v','libx264','-preset','veryfast','-an',filter_src])
    hevc=w/'hevc.mp4'
    run(['ffmpeg','-nostdin','-y','-v','error','-f','lavfi','-i','testsrc2=size=1920x1080:rate=30','-frames:v','15','-pix_fmt','yuv420p','-c:v','libx265','-preset','ultrafast','-x265-params','log-level=error','-an',hevc])
    # Many tiny VFR packets: too few → process-start noise; too many → shared demux dilutes wrapper delta.
    vfr=w/'vfr.mkv'
    run(['ffmpeg','-nostdin','-y','-v','error','-f','lavfi','-i','testsrc2=size=160x90:rate=30','-frames:v','1500','-vf','setpts=N+floor(N/2)','-fps_mode','vfr','-c:v','ffv1','-level','3',vfr])
    a=w/'a.mp4';b=w/'b.mp4';fixture(a,'3',True);fixture(b,'3',True)
    concat_list=w/'concat.txt';concat_list.write_text(f"file '{a.as_posix()}'\nfile '{b.as_posix()}'\n")
    # Few-frame FFV1+AAC interval: longer clips bury wrapper delta under shared decode/encode.
    av_aac=w/'av-aac.mkv'
    run(['ffmpeg','-nostdin','-y','-v','error','-f','lavfi','-i','testsrc2=size=160x90:rate=25','-f','lavfi','-i','sine=frequency=997:sample_rate=48000','-frames:v','6','-pix_fmt','yuv420p','-c:v','ffv1','-level','3','-g','1','-c:a','aac','-b:a','128k',av_aac])
    vf_av_aac='trim=start=0.04:end=0.16,setpts=PTS-STARTPTS'
    af_av_aac='atrim=start_sample=1920:end_sample=7680,asetpts=PTS-STARTPTS'
    # Large SRT remux: tiny cues collapse to process-start noise on a quiet host.
    captions=w/'captions.srt'
    captions.write_text(''.join(
        f'{i}\n00:{i//60:02d}:{i%60:02d},000 --> 00:{i//60:02d}:{i%60:02d},400\ncue {i}\n\n'
        for i in range(1, 4001)
    ))
    convert_captions=w/'captions-convert.srt'
    convert_captions.write_text(''.join(
        f'{i}\n00:{i//60:02d}:{i%60:02d},000 --> 00:{i//60:02d}:{i%60:02d},400\ncue {i} text line\n\n'
        for i in range(1, 4001)
    ))
    # Few-frame burn fixture keeps wrapper delta above shared libass work.
    burn_src=w/'burn.mp4'
    run(['ffmpeg','-nostdin','-y','-v','error','-f','lavfi','-i','testsrc2=size=1280x720:rate=25','-frames:v','5','-pix_fmt','yuv420p','-c:v','libx264','-preset','veryfast','-an',burn_src])
    burn_subs=w/'burn.srt';burn_subs.write_text('1\n00:00:00,000 --> 00:00:00,400\nHELLO\n')
    burn_path=str(burn_subs.resolve()).replace('\\','/')
    if burn_path.startswith('//?/'):
        burn_path=burn_path[4:]
    vf_burn="subtitles='"+burn_path.replace(':','\\:')+"'"
    overlay_src=w/'overlay-main.mp4'
    overlay_pip=w/'overlay-pip.mp4'
    run(['ffmpeg','-nostdin','-y','-v','error','-f','lavfi','-i','testsrc2=size=1280x720:rate=25','-frames:v','5','-pix_fmt','yuv420p','-c:v','libx264','-preset','veryfast','-an',overlay_src])
    run(['ffmpeg','-nostdin','-y','-v','error','-f','lavfi','-i','testsrc2=size=160x90:rate=25','-frames:v','5','-pix_fmt','yuv420p','-c:v','libx264','-preset','veryfast','-an',overlay_pip])
    ov_path=str(overlay_pip.resolve()).replace('\\','/')
    if ov_path.startswith('//?/'):
        ov_path=ov_path[4:]
    vf_overlay="movie='"+ov_path.replace(':','\\:')+"'[ov];[in][ov]overlay=32:24"
    meta_src=w/'meta-src.mp4'
    run(['ffmpeg','-nostdin','-y','-v','error','-i',medium,'-metadata','title=Original','-c','copy','-an',meta_src])
    audio_src=w/'audio.m4a'
    run(['ffmpeg','-nostdin','-y','-v','error','-f','lavfi','-i','sine=frequency=997:sample_rate=48000','-t','0.25','-ac','2','-c:a','aac','-b:a','192k',audio_src])
    volume_src=w/'volume.m4a'
    run(['ffmpeg','-nostdin','-y','-v','error','-f','lavfi','-i','sine=frequency=997:sample_rate=48000','-t','0.35','-ac','2','-c:a','aac','-b:a','192k',volume_src])
    # Resample: ultra-short clip maximizes wrapper delta (longer AAC decode equalizes).
    resample_src=w/'resample.m4a'
    run(['ffmpeg','-nostdin','-y','-v','error','-f','lavfi','-i','sine=frequency=997:sample_rate=48000','-t','0.06','-ac','2','-c:a','aac','-b:a','192k',resample_src])
    mix_a=w/'mix-a.m4a';mix_b=w/'mix-b.m4a';mix_c=w/'mix-c.m4a'
    run(['ffmpeg','-nostdin','-y','-v','error','-f','lavfi','-i','sine=frequency=440:sample_rate=48000','-t','15','-ac','2','-c:a','aac','-b:a','192k',mix_a])
    run(['ffmpeg','-nostdin','-y','-v','error','-f','lavfi','-i','sine=frequency=880:sample_rate=48000','-t','15','-ac','2','-c:a','aac','-b:a','192k',mix_b])
    run(['ffmpeg','-nostdin','-y','-v','error','-f','lavfi','-i','sine=frequency=660:sample_rate=48000','-t','15','-ac','2','-c:a','aac','-b:a','192k',mix_c])
    vf_amix='[0:a][1:a]amix=inputs=2:duration=shortest:dropout_transition=0:normalize=1'
    vf_amix3='[0:a][1:a][2:a]amix=inputs=3:duration=shortest:dropout_transition=0:weights=1 2 0.5:normalize=1,aformat=sample_fmts=flt'
    merge_l=w/'merge-l.m4a';merge_r=w/'merge-r.m4a'
    run(['ffmpeg','-nostdin','-y','-v','error','-f','lavfi','-i','sine=frequency=440:sample_rate=48000','-t','15','-ac','1','-c:a','aac','-b:a','192k',merge_l])
    run(['ffmpeg','-nostdin','-y','-v','error','-f','lavfi','-i','sine=frequency=880:sample_rate=48000','-t','15','-ac','1','-c:a','aac','-b:a','192k',merge_r])
    vf_amerge='[0:a][1:a]amerge=inputs=2'
    crop='240:134:960:540';vf_crop='crop=960:540:240:134'
    scale='960:540';vf_scale='scale=960:540:flags=neighbor'
    transpose='clock';vf_transpose='transpose=clock'
    pad='2048:1152:64:36';vf_pad='pad=2048:1152:64:36:black'
    # 1920x1080 @ 30° → rotw/roth trunc sizes used by both sides.
    rotate='30';vf_rotate='rotate=a=30*PI/180:ow=2202:oh=1895:c=black'
    outputs={
      name:(w/f'{name}-f.{extension}',w/f'{name}-x.{extension}')
      for name,extension in [('subtitle_remux','mkv'),('subtitle_convert','mkv'),('burn','mkv'),('vfr_identity','mkv'),('copy','mp4'),('metadata','mp4'),('trim','mp4'),('bframe_trim','mp4'),('hevc_trim','mp4'),('midgop_trim','mp4'),('concat','mp4'),('resample','wav'),('channels','wav'),('volume','wav'),('amix','wav'),('amix3','wav'),('amerge','wav'),('av_aac_interval','mkv')]
    }
    pairs={
      'subtitle_remux':([binary,'media','remux',captions,outputs['subtitle_remux'][0],'--quiet'],['ffmpeg','-nostdin','-y','-v','error','-i',captions,'-c','copy','-an',outputs['subtitle_remux'][1]]),
      'subtitle_convert':([binary,'media','convert-subtitles',convert_captions,outputs['subtitle_convert'][0],'--codec','ass','--quiet'],['ffmpeg','-nostdin','-y','-v','error','-i',convert_captions,'-c:s','ass','-an',outputs['subtitle_convert'][1]]),
      'burn':([binary,'media','decode',burn_src,'--subs',burn_subs,'--quiet'],['ffmpeg','-nostdin','-v','error','-i',burn_src,'-vf',vf_burn,'-an','-f','null','-']),
      'overlay':([binary,'media','decode',overlay_src,'--overlay',overlay_pip,'--overlay-x','32','--overlay-y','24','--quiet'],['ffmpeg','-nostdin','-v','error','-i',overlay_src,'-vf',vf_overlay,'-an','-f','null','-']),
      'vfr_identity':([binary,'media','transcode-lossless',vfr,outputs['vfr_identity'][0],'--quiet'],['ffmpeg','-nostdin','-y','-v','error','-i',vfr,'-c','copy','-an',outputs['vfr_identity'][1]]),
      'copy':([binary,'media','remux',medium,outputs['copy'][0],'--quiet'],['ffmpeg','-nostdin','-y','-v','error','-i',medium,'-c','copy','-an',outputs['copy'][1]]),
      'metadata':([binary,'media','remux',meta_src,outputs['metadata'][0],'--metadata','title=Renamed','--quiet'],['ffmpeg','-nostdin','-y','-v','error','-i',meta_src,'-c','copy','-metadata','title=Renamed','-an',outputs['metadata'][1]]),
      'crop':([binary,'media','decode',filter_src,'--crop',crop,'--quiet'],['ffmpeg','-nostdin','-v','error','-i',filter_src,'-vf',vf_crop,'-an','-f','null','-']),
      'hflip':([binary,'media','decode',filter_src,'--hflip','--quiet'],['ffmpeg','-nostdin','-v','error','-i',filter_src,'-vf','hflip','-an','-f','null','-']),
      'vflip':([binary,'media','decode',filter_src,'--vflip','--quiet'],['ffmpeg','-nostdin','-v','error','-i',filter_src,'-vf','vflip','-an','-f','null','-']),
      'scale':([binary,'media','decode',filter_src,'--scale',scale,'--quiet'],['ffmpeg','-nostdin','-v','error','-i',filter_src,'-vf',vf_scale,'-an','-f','null','-']),
      'transpose':([binary,'media','decode',filter_src,'--transpose',transpose,'--quiet'],['ffmpeg','-nostdin','-v','error','-i',filter_src,'-vf',vf_transpose,'-an','-f','null','-']),
      'pad':([binary,'media','decode',filter_src,'--pad',pad,'--quiet'],['ffmpeg','-nostdin','-v','error','-i',filter_src,'-vf',vf_pad,'-an','-f','null','-']),
      'rotate':([binary,'media','decode',rotate_src,'--rotate',rotate,'--quiet'],['ffmpeg','-nostdin','-v','error','-i',rotate_src,'-vf',vf_rotate,'-an','-f','null','-']),
      'resample':([binary,'media','decode-audio',resample_src,outputs['resample'][0],'--rate','44100','--quiet'],['ffmpeg','-nostdin','-y','-v','error','-i',resample_src,'-ar','44100','-c:a','pcm_f32le',outputs['resample'][1]]),
      'channels':([binary,'media','decode-audio',audio_src,outputs['channels'][0],'--channels','1','--quiet'],['ffmpeg','-nostdin','-y','-v','error','-i',audio_src,'-ac','1','-c:a','pcm_f32le',outputs['channels'][1]]),
      'volume':([binary,'media','decode-audio',volume_src,outputs['volume'][0],'--volume','0.5','--quiet'],['ffmpeg','-nostdin','-y','-v','error','-i',volume_src,'-af','volume=0.5','-c:a','pcm_f32le',outputs['volume'][1]]),
      'amix':([binary,'media','mix-audio',outputs['amix'][0],mix_a,mix_b,'--quiet'],['ffmpeg','-nostdin','-y','-v','error','-i',mix_a,'-i',mix_b,'-filter_complex',vf_amix,'-c:a','pcm_f32le',outputs['amix'][1]]),
      'amix3':([binary,'media','mix-audio',outputs['amix3'][0],mix_a,mix_b,mix_c,'--weights','1,2,0.5','--quiet'],['ffmpeg','-nostdin','-y','-v','error','-i',mix_a,'-i',mix_b,'-i',mix_c,'-filter_complex',vf_amix3,'-c:a','pcm_f32le',outputs['amix3'][1]]),
      'amerge':([binary,'media','merge-audio',outputs['amerge'][0],merge_l,merge_r,'--quiet'],['ffmpeg','-nostdin','-y','-v','error','-i',merge_l,'-i',merge_r,'-filter_complex',vf_amerge,'-c:a','pcm_f32le',outputs['amerge'][1]]),
      'fused':([binary,'media','decode',short,'--crop',crop,'--hflip','--vflip','--quiet'],['ffmpeg','-nostdin','-v','error','-i',short,'-vf',vf_crop+',hflip,vflip','-an','-f','null','-']),
      'hevc_decode':([binary,'media','decode',hevc,'--quiet'],['ffmpeg','-nostdin','-v','error','-i',hevc,'-an','-f','null','-']),
      'cut':([binary,'media','decode',gop,'--from','2','--to','3','--quiet'],['ffmpeg','-nostdin','-v','error','-ss','2','-to','3','-i',gop,'-an','-f','null','-']),
      'trim':([binary,'media','trim',gop,outputs['trim'][0],'--from','8','--to','10','--quiet'],['ffmpeg','-nostdin','-y','-v','error','-i',gop,'-ss','8','-to','10','-c','copy','-an',outputs['trim'][1]]),
      'bframe_trim':([binary,'media','trim',bgop,outputs['bframe_trim'][0],'--from','2','--to','3','--quiet'],['ffmpeg','-nostdin','-y','-v','error','-ss','2','-i',bgop,'-frames:v','30','-c','copy','-an',outputs['bframe_trim'][1]]),
      'hevc_trim':([binary,'media','trim',hgop,outputs['hevc_trim'][0],'--from','8','--to','40','--quiet'],['ffmpeg','-nostdin','-y','-v','error','-ss','8','-i',hgop,'-t','32','-c','copy',outputs['hevc_trim'][1]]),
      'midgop_trim':([binary,'media','trim',bgop,outputs['midgop_trim'][0],'--from','2.1','--to','3.7','--quiet'],['ffmpeg','-nostdin','-y','-v','error','-ss','2.1','-i',bgop,'-to','3.7','-c','copy','-an',outputs['midgop_trim'][1]]),
      'concat':([binary,'media','concat',outputs['concat'][0],a,b,'--quiet'],['ffmpeg','-nostdin','-y','-v','error','-f','concat','-safe','0','-i',concat_list,'-c','copy','-an',outputs['concat'][1]]),
      'av_aac_interval':([binary,'media','transcode-lossless',av_aac,outputs['av_aac_interval'][0],'--from','0.04','--to','0.16','--quiet'],['ffmpeg','-nostdin','-y','-v','error','-i',av_aac,'-vf',vf_av_aac,'-af',af_av_aac,'-c:v','ffv1','-level','3','-c:a','pcm_f32le',outputs['av_aac_interval'][1]]),
      'decode':([binary,'media','decode',filter_src,'--quiet'],['ffmpeg','-nostdin','-v','error','-i',filter_src,'-an','-f','null','-']),
    }
    samples={name:{'fvid':[],'ffmpeg':[]}for name in pairs};rng=random.Random(20260917)
    def cleanup(name):
        for path in outputs.get(name,()):
            path.unlink(missing_ok=True)
    for name,(fvid,ffmpeg) in pairs.items():
        run(fvid);cleanup(name);run(ffmpeg);cleanup(name)
    for _ in range(args.runs):
        names=list(pairs);rng.shuffle(names)
        for name in names:
            variants=[('fvid',pairs[name][0]),('ffmpeg',pairs[name][1])];rng.shuffle(variants)
            for variant,command in variants:
                start=time.perf_counter_ns();run(command);samples[name][variant].append((time.perf_counter_ns()-start)/1e6);cleanup(name)
    results={}
    failures=[]
    for name,values in samples.items():
        medians={variant:statistics.median(items)for variant,items in values.items()}
        # Paired within-round deltas remove thermal/order bias (same as GPU gate).
        paired=[1-f/x for f,x in zip(values['fvid'],values['ffmpeg'])]
        delta=statistics.median(paired)
        results[name]=dict(median_ms=medians,delta_vs_ffmpeg=delta,median_paired_delta_vs_ffmpeg=delta,paired_deltas=paired,samples_ms=values)
        print(f'{name}: {medians["fvid"]:.3f} vs {medians["ffmpeg"]:.3f} ms, paired delta={delta:+.1%}')
        if delta<=args.min_delta:failures.append(f'{name} {delta:+.1%}')
    report=dict(created_utc=datetime.datetime.now(datetime.timezone.utc).isoformat(),scope=f'{args.runs} randomized rounds; pair order and operation order shuffled; paired within-round deltas',minimum_delta=args.min_delta,validation_report='benchmarks/media-validation.json',binary_sha256=hashlib.sha256(binary.read_bytes()).hexdigest(),results=results,status='failed' if failures else 'passed')
    (ROOT/'benchmarks/cpu-media-gate.json').write_text(json.dumps(report,indent=2)+'\n')
    if failures:raise SystemExit('strict CPU media gate failed: '+', '.join(failures))
