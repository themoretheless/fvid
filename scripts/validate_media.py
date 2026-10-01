#!/usr/bin/env python3
"""Native media end-to-end qualification; FFmpeg CLI is only a test oracle/generator."""
import argparse,hashlib,json,pathlib,subprocess,tempfile,datetime
from common import ROOT, release_binary
BINARY=release_binary()

def run(command):
    result=subprocess.run([str(v) for v in command],capture_output=True,timeout=180)
    if result.returncode:raise RuntimeError(f'{command}: {result.stderr.decode(errors="replace")}')
    return result

def ff(*args):return run(['ffmpeg','-nostdin','-v','error',*args])
def native(*args):return json.loads(run([BINARY,'media',*args]).stdout)
def packets(path,stream='v:0'):
    data=json.loads(run(['ffprobe','-v','error','-select_streams',stream,'-show_packets','-show_data_hash','sha256','-show_entries','packet=data_hash,pts_time,dts_time,duration_time','-of','json',path]).stdout)
    return data['packets']
def hashes(path,stream='v:0'):return [p['data_hash'] for p in packets(path,stream)]
def pixels(path,filters=None,pixfmt='yuv420p'):
    args=['-i',path,'-map','0:v:0','-an']
    if filters:args+=['-vf',filters]
    return ff(*args,'-fps_mode','passthrough','-pix_fmt',pixfmt,'-f','rawvideo','-').stdout
def movie_overlay_vf(path,x=0,y=0):
    p=str(pathlib.Path(path).resolve()).replace('\\','/')
    if p.startswith('//?/'):p=p[4:]
    return "movie='"+p.replace(':','\\:')+f"'[ov];[in][ov]overlay={x}:{y}"
def framepack_oracle(args):
    fa=f'framepack={args}' if args else 'framepack'
    return f"[0:v]split[leftsel][rightsel];[leftsel]select='not(mod(n,2))',setpts=N/FRAME_RATE/TB[left];[rightsel]select='mod(n,2)',setpts=N/FRAME_RATE/TB[right];[left][right]{fa}"
def framepack_pixels(path,args):
    return ff('-i',path,'-filter_complex',framepack_oracle(args),'-an','-fps_mode','passthrough','-pix_fmt','yuv420p','-f','rawvideo','-').stdout
def telecine_oracle(args):
    return f'telecine={args}' if args else 'telecine'
def telecine_pixels(path,args):
    return ff('-i',path,'-vf',telecine_oracle(args),'-an','-fps_mode','passthrough','-pix_fmt','yuv420p','-f','rawvideo','-').stdout
def pullup_oracle(args):
    return f'pullup={args}' if args else 'pullup'
def pullup_pixels(path,args):
    return ff('-i',path,'-vf',pullup_oracle(args),'-an','-fps_mode','passthrough','-pix_fmt','yuv420p','-f','rawvideo','-').stdout
def decimate_oracle(args):
    return f'decimate={args}' if args else 'decimate'
def decimate_pixels(path,args):
    return ff('-i',path,'-vf',decimate_oracle(args),'-an','-fps_mode','passthrough','-pix_fmt','yuv420p','-f','rawvideo','-').stdout
def mpdecimate_oracle(args):
    return f'mpdecimate={args}' if args else 'mpdecimate'
def mpdecimate_pixels(path,args):
    return ff('-i',path,'-vf',mpdecimate_oracle(args),'-an','-fps_mode','passthrough','-pix_fmt','yuv420p','-f','rawvideo','-').stdout
def framestep_oracle(args):
    return f'framestep={args}' if args else 'framestep'
def framestep_pixels(path,args):
    return ff('-i',path,'-vf',framestep_oracle(args),'-an','-fps_mode','passthrough','-pix_fmt','yuv420p','-f','rawvideo','-').stdout
def tile_oracle(args):
    return f'tile={args}' if args else 'tile'
def tile_pixels(path,args):
    return ff('-i',path,'-vf',tile_oracle(args),'-an','-fps_mode','passthrough','-pix_fmt','yuv420p','-f','rawvideo','-').stdout
def untile_oracle(args):
    return f'untile={args}' if args else 'untile'
def untile_pixels(path,args):
    return ff('-i',path,'-vf',untile_oracle(args),'-an','-fps_mode','passthrough','-pix_fmt','yuv420p','-f','rawvideo','-').stdout
def shuffleframes_oracle(args):
    return f'shuffleframes={args}' if args else 'shuffleframes'
def shuffleframes_pixels(path,args):
    return ff('-i',path,'-vf',shuffleframes_oracle(args),'-an','-fps_mode','passthrough','-pix_fmt','yuv420p','-f','rawvideo','-').stdout
def reverse_oracle(args):
    return f'reverse={args}' if args else 'reverse'
def reverse_pixels(path,args):
    return ff('-i',path,'-vf',reverse_oracle(args),'-an','-fps_mode','passthrough','-pix_fmt','yuv420p','-f','rawvideo','-').stdout
def loop_oracle(args):
    return f'loop={args}' if args else 'loop'
def loop_pixels(path,args):
    return ff('-i',path,'-vf',loop_oracle(args),'-an','-fps_mode','passthrough','-pix_fmt','yuv420p','-f','rawvideo','-').stdout
def thumbnail_oracle(args):
    return f'thumbnail={args}' if args else 'thumbnail'
def thumbnail_pixels(path,args):
    return ff('-i',path,'-vf',thumbnail_oracle(args),'-an','-fps_mode','passthrough','-pix_fmt','yuv420p','-f','rawvideo','-').stdout
def audio(path):return ff('-i',path,'-map','0:a:0','-vn','-c:a','pcm_s16le','-f','s16le','-').stdout
def video_timeline(path):
    data=json.loads(run(['ffprobe','-v','error','-select_streams','v:0','-show_frames','-show_streams','-show_entries','stream=time_base,avg_frame_rate:frame=pts,duration','-of','json',path]).stdout)
    return dict(streams=data['streams'],frames=[(f.get('pts'),f.get('duration')) for f in data['frames']])

def main():
    global BINARY
    parser=argparse.ArgumentParser()
    parser.add_argument('--binary',default=str(BINARY))
    args=parser.parse_args()
    BINARY=pathlib.Path(args.binary).resolve()
    checks=[]
    def record(name,**details):checks.append(dict(name=name,status='passed',**details));print(name,flush=True)
    with tempfile.TemporaryDirectory(prefix='fvid-media-')as temp:
        w=pathlib.Path(temp); source=w/'source.mp4'
        ff('-f','lavfi','-i','testsrc2=size=128x72:rate=25','-t','2','-an','-c:v','libx264','-bf','0','-g','25','-keyint_min','25','-sc_threshold','0','-pix_fmt','yuv420p',source)
        info=native('probe',source);assert info['streams'][0]['codec']=='h264'
        assert info['streams'][0]['average_frame_rate']==[25,1]
        assert info['streams'][0]['profile'] and info['streams'][0]['level']>0
        assert info['bit_rate']>0 and info['duration_us']==2000000
        record('rich probe H264 profile/rate/timeline',info=info)
        copied=w/'remux.mkv';stats=native('remux',source,copied);assert hashes(source)==hashes(copied);assert pixels(source)==pixels(copied);record('remux MP4 to MKV packet and decoded equality',stats=stats)
        ts=w/'source.ts';ff('-i',source,'-c','copy','-f','mpegts',ts)
        out=w/'ts-remux.mkv';stats=native('remux',ts,out);assert pixels(out)==pixels(source);record('MPEG-TS remux to MKV preserves decoded pixels',stats=stats)
        mov=w/'source.mov';ff('-i',source,'-c','copy',mov)
        out=w/'mov-remux.mkv';stats=native('remux',mov,out);assert pixels(out)==pixels(source);assert hashes(out)==hashes(source);record('MOV remux to MKV preserves packets and pixels',stats=stats)
        result=subprocess.run([str(BINARY),'media','remux',str(source),str(w/'progress.mkv'),'--progress'],capture_output=True,timeout=60)
        assert result.returncode==0 and (w/'progress.mkv').exists()
        progress_lines=[json.loads(line) for line in result.stderr.decode().splitlines() if line.strip().startswith('{')]
        assert progress_lines and progress_lines[-1].get('done') is True and progress_lines[-1]['packets']>0
        record('remux --progress emits NDJSON samples ending with done',progress=progress_lines[-1])
        trimmed=w/'trim.mp4';stats=native('trim',source,trimmed,'--from','1','--to','2');assert hashes(trimmed)==hashes(source)[25:];assert pixels(trimmed)==pixels(source)[25*128*72*3//2:];record('exact IDR trim',stats=stats)
        stats=native('decode',source,'--from','1','--to','2')
        assert stats['video_frames']==25;record('no-reorder decode interval stops before end packet',stats=stats)
        concatenated=w/'concat.mp4';stats=native('concat',concatenated,source,source);assert hashes(concatenated)==hashes(source)*2;assert pixels(concatenated)==pixels(source)*2;record('concat H264 two segments',stats=stats)
        out=w/'crop.mkv';stats=native('crop-lossless',source,out,'--crop','2:2:64:48');assert pixels(out)==pixels(source,'crop=64:48:2:2:exact=1');record('H264 decode -> borrowed crop -> FFV1',stats=stats)
        bframes=w/'bframes.mp4';ff('-f','lavfi','-i','testsrc2=size=128x72:rate=25','-t','2','-c:v','libx264','-bf','3',bframes)
        out=w/'bframes-crop.mkv';stats=native('crop-lossless',bframes,out,'--crop','2:2:64:48');assert pixels(out)==pixels(bframes,'crop=64:48:2:2:exact=1');record('B-frame decoder drain and lossless crop',stats=stats)
        stats=native('decode',bframes,'--from','0.12','--to','0.52')
        assert stats['video_frames']==10;record('reordered decode interval preserves frame-PTS boundary',stats=stats)
        for start,end,flags,reference,count in [
            ('0.12','0.52',[],'trim=start=0.12:end=0.52',10),
            ('0.12','0.52',['--crop','2:2:64:48','--vflip'],'trim=start=0.12:end=0.52,crop=64:48:2:2:exact=1,vflip',10),
            ('1.88','2',[],'trim=start=1.88:end=2',3),
        ]:
            out=w/'lossless-trim.mkv';stats=native('transcode-lossless',bframes,out,'--from',start,'--to',end,*flags)
            assert stats['video_frames']==count
            assert pixels(out)==pixels(bframes,reference)
            timing=packets(out);assert float(timing[0]['pts_time'])==0
            assert all(abs(float(p['pts_time'])-i/25)<0.000001 for i,p in enumerate(timing))
            record(f'B-frame lossless interval {start}:{end} {flags}',stats=stats);out.unlink()
        seek_source=w/'seek.mp4';ff('-f','lavfi','-i','testsrc2=size=128x72:rate=25','-t','4','-c:v','libx264','-bf','3','-g','25','-keyint_min','25','-sc_threshold','0',seek_source)
        out=w/'closed-b-trim.mp4';stats=native('trim',seek_source,out,'--from','1','--to','2')
        assert hashes(out)==hashes(seek_source)[25:50]
        assert pixels(out)==pixels(seek_source)[25*128*72*3//2:50*128*72*3//2]
        assert float(packets(out)[0]['pts_time'])==0
        record('closed-GOP H.264 IDR trim keeps B-frames between IDRs',stats=stats)
        out=w/'midgop-trim.mp4';stats=native('trim',seek_source,out,'--from','1.12','--to','2')
        assert pixels(out)==pixels(seek_source,'trim=start=1.12:end=2')
        assert float(packets(out)[0]['pts_time'])<0
        record('closed-GOP H.264 mid-GOP trim pre-roll with presentation window',stats=stats)
        out=w/'midgop-both.mp4';stats=native('trim',seek_source,out,'--from','1.12','--to','1.52')
        assert pixels(out)==pixels(seek_source,'trim=start=1.12:end=1.52')
        dur=float(json.loads(run(['ffprobe','-v','error','-show_entries','format=duration','-of','json',out]).stdout)['format']['duration'])
        assert abs(dur-0.4)<0.000001
        record('closed-GOP H.264 mid-GOP start and end with post-roll discard',stats=stats)
        hevc_gop=w/'hevc-gop.mp4';ff('-f','lavfi','-i','testsrc2=size=128x72:rate=25','-t','4','-pix_fmt','yuv420p','-c:v','libx265','-preset','ultrafast','-x265-params','keyint=25:min-keyint=25:scenecut=0:bframes=3:b-adapt=0:open-gop=0:log-level=error',hevc_gop)
        out=w/'closed-hevc-trim.mp4';stats=native('trim',hevc_gop,out,'--from','1','--to','2')
        assert hashes(out)==hashes(hevc_gop)[25:50]
        assert pixels(out)==pixels(hevc_gop)[25*128*72*3//2:50*128*72*3//2]
        assert float(packets(out)[0]['pts_time'])==0
        record('closed-GOP HEVC IRAP trim keeps B-frames between IRAPs',stats=stats)
        out=w/'midgop-hevc-trim.mp4';stats=native('trim',hevc_gop,out,'--from','1.12','--to','2')
        assert pixels(out)==pixels(hevc_gop,'trim=start=1.12:end=2')
        assert float(packets(out)[0]['pts_time'])<0
        record('closed-GOP HEVC mid-GOP trim pre-roll with presentation window',stats=stats)
        out=w/'midgop-hevc-both.mp4';stats=native('trim',hevc_gop,out,'--from','1.12','--to','1.52')
        assert pixels(out)==pixels(hevc_gop,'trim=start=1.12:end=1.52')
        record('closed-GOP HEVC mid-GOP start and end with post-roll discard',stats=stats)
        open_gop=w/'open-gop.mp4';ff('-f','lavfi','-i','testsrc2=size=128x72:rate=25','-t','4','-c:v','libx264','-bf','3','-g','25','-keyint_min','25','-x264-params','open-gop=1',open_gop)
        out=w/'open-gop-trim.mp4';stats=native('trim',open_gop,out,'--from','0','--to','1')
        assert pixels(out)==pixels(open_gop,'trim=start=0:end=1')
        record('open-GOP H.264 trim from IDR to open keyframe end',stats=stats)
        out=w/'open-gop-start.mp4';stats=native('trim',open_gop,out,'--from','1','--to','2')
        assert pixels(out)==pixels(open_gop,'trim=start=1:end=2')
        assert stats['packets']>=40
        ignored=json.loads(run(['ffprobe','-v','error','-f','mp4','-ignore_editlist','1','-select_streams','v:0','-count_packets','-show_entries','stream=nb_read_packets','-of','json',out]).stdout)
        assert int(ignored['streams'][0]['nb_read_packets'])>=40
        record('open-GOP H.264 start via preceding-IDR stream-copy pre-roll',stats=stats)
        out=w/'open-gop-lossless.mkv';stats=native('transcode-lossless',open_gop,out,'--from','1','--to','2')
        assert pixels(out)==pixels(open_gop,'trim=start=1:end=2')
        record('open-GOP H.264 start via lossless interval is sample-exact',stats=stats)
        aac=w/'aac-delay.m4a';ff('-f','lavfi','-i','sine=frequency=997:sample_rate=48000','-t','2','-c:a','aac','-b:a','128k',aac)
        out=w/'aac-trim.m4a';stats=native('trim',aac,out,'--from','0','--to','0.064')
        assert audio(out)==ff('-i',aac,'-af','atrim=0:0.064','-c:a','pcm_s16le','-f','s16le','-').stdout
        pad=json.loads(run(['ffprobe','-v','error','-select_streams','a:0','-show_entries','stream=initial_padding','-of','json',out]).stdout)['streams'][0]['initial_padding']
        assert int(pad)==1024
        record('AAC trim from 0 preserves priming delay and sample-exact PCM',stats=stats)
        out=w/'aac-mid.m4a';stats=native('trim',aac,out,'--from','0.064','--to','0.128')
        ref=w/'aac-mid-ff.m4a';ff('-ss','0.064','-i',aac,'-t','0.064','-c','copy',ref)
        assert audio(out)==audio(ref)
        record('AAC mid-stream packet-aligned trim matches FFmpeg stream copy',stats=stats)
        out=w/'aac-seam.wav';stats=native('trim',aac,out,'--from','0.01','--to','0.05')
        assert audio(out)==ff('-i',aac,'-af','atrim=0.01:0.05,asetpts=PTS-STARTPTS','-c:a','pcm_s16le','-f','s16le','-').stdout
        assert 'seam' in stats['backend']
        record('AAC non-aligned trim decodes sample-exact PCM',stats=stats)
        joined=w/'aac-concat.m4a';stats=native('concat',joined,aac,aac)
        lst=w/'aac-concat.txt';lst.write_text(f"file '{aac.as_posix()}'\nfile '{aac.as_posix()}'\n")
        ref=w/'aac-concat-ff.m4a';ff('-f','concat','-safe','0','-i',lst,'-c','copy',ref)
        assert audio(joined)==audio(ref)
        record('AAC concat preserves matching initial_padding',stats=stats)
        for start,end in [('2.12','2.52'),('3.88','4')]:
            full=w/'full-trim.mkv';fast=w/'seek-trim.mkv'
            baseline=native('transcode-lossless',seek_source,full,'--from',start,'--to',end)
            stats=native('transcode-lossless',seek_source,fast,'--from',start,'--to',end,'--seek')
            assert pixels(fast)==pixels(full)==pixels(seek_source,f'trim=start={start}:end={end}')
            assert [p['pts_time'] for p in packets(fast)]==[p['pts_time'] for p in packets(full)]
            assert stats['decoded_frames']<baseline['decoded_frames'] and stats['seek_used']
            record(f'keyframe seek B-frame interval {start}:{end}',stats=stats,baseline=baseline)
            full.unlink();fast.unlink()
        av=w/'av.mkv';ff('-f','lavfi','-i','testsrc2=size=128x72:rate=25','-f','lavfi','-i','sine=frequency=997:sample_rate=48000','-t','2','-c:v','ffv1','-level','3','-g','1','-af','asetnsamples=n=1920:p=0','-c:a','pcm_s16le',av)
        out=w/'av-seek.mkv';full=w/'av-full.mkv'
        baseline=native('transcode-lossless',av,full,'--from','0.125','--to','0.525')
        stats=native('transcode-lossless',av,out,'--from','0.125','--to','0.525','--seek')
        assert pixels(out)==pixels(full)==pixels(av,'trim=start=0.125:end=0.525')
        assert audio(out)==audio(full)==audio(av)[6000*2:25200*2]
        assert stats['seek_used'] and stats['decoded_frames']<=baseline['decoded_frames']
        record('PCM audio seek keeps sample-exact lossless interval',stats=stats,baseline=baseline)
        full.unlink();out.unlink()
        out=w/'av-crop.mkv';stats=native('crop-lossless',av,out,'--crop','2:2:64:48');assert pixels(out)==pixels(av,'crop=64:48:2:2:exact=1');assert audio(out)==audio(av);record('lossless crop preserves PCM audio samples',stats=stats)
        out=w/'av-sample-trim.mkv';stats=native('transcode-lossless',av,out,'--from','0.125','--to','0.525')
        assert pixels(out)==pixels(av,'trim=start=0.125:end=0.525')
        assert audio(out)==audio(av)[6000*2:25200*2]
        assert stats['trimmed_audio_sample_frames']==19200
        record('lossless interval cuts inside PCM packets without sample changes',stats=stats)
        av_aac=w/'av-aac.mkv';ff('-f','lavfi','-i','testsrc2=size=128x72:rate=25','-f','lavfi','-i','sine=frequency=997:sample_rate=48000','-t','2','-pix_fmt','yuv420p','-c:v','ffv1','-level','3','-g','1','-c:a','aac','-b:a','128k',av_aac)
        out=w/'av-aac-trim.mkv';stats=native('transcode-lossless',av_aac,out,'--from','0.125','--to','0.525')
        assert pixels(out)==pixels(av_aac,'trim=start=0.125:end=0.525')
        assert audio(out)==ff('-i',av_aac,'-af','atrim=start_sample=6000:end_sample=25200,asetpts=PTS-STARTPTS','-c:a','pcm_s16le','-f','s16le','-').stdout
        assert stats['trimmed_audio_sample_frames']==19200 and stats['copied_packets']==0
        record('lossless interval decodes AAC with sample-exact PCM beside video',stats=stats)
        out=w/'av-aac-seam.mkv';stats=native('trim',av_aac,out,'--from','0.04','--to','0.12')
        assert pixels(out)==pixels(av_aac,'trim=start=0.04:end=0.12')
        assert audio(out)==ff('-i',av_aac,'-af','atrim=start_sample=1920:end_sample=5760,asetpts=PTS-STARTPTS','-c:a','pcm_s16le','-f','s16le','-').stdout
        assert 'seam' in stats['backend'] and stats['packets']>0
        record('A/V trim keeps video stream copy beside non-aligned AAC→PCM',stats=stats)
        dual=w/'dual-video.mkv'
        ff('-f','lavfi','-i','testsrc2=size=128x72:rate=25','-f','lavfi','-i','testsrc2=size=96x54:rate=25','-t','0.4','-map','0:v','-map','1:v','-c:v','ffv1','-level','3','-g','1',dual)
        out=w/'dual-crop.mkv';stats=native('transcode-lossless',dual,out,'--crop','2:2:64:48','--streams','0,1')
        assert pixels(out)==pixels(dual,'crop=64:48:2:2:exact=1')
        assert hashes(out,'v:1')==hashes(dual,'v:1')
        assert stats['copied_packets']>0
        record('lossless crop remuxes secondary video stream',stats=stats)
        out=w/'dual-interval.mkv';stats=native('transcode-lossless',dual,out,'--from','0.08','--to','0.28','--streams','0,1')
        assert pixels(out)==pixels(dual,'trim=start=0.08:end=0.28')
        secondary=ff('-i',dual,'-map','0:v:1','-vf','trim=start=0.08:end=0.28,setpts=PTS-STARTPTS','-fps_mode','passthrough','-pix_fmt','yuv420p','-f','rawvideo','-').stdout
        actual=ff('-i',out,'-map','0:v:1','-an','-fps_mode','passthrough','-pix_fmt','yuv420p','-f','rawvideo','-').stdout
        assert actual==secondary and stats['copied_packets']>0
        record('lossless interval remuxes secondary non-reordered video',stats=stats)
        full=w/'dual-full.mkv';fast=w/'dual-seek.mkv'
        baseline=native('transcode-lossless',dual,full,'--from','0.08','--to','0.28','--streams','0,1')
        stats=native('transcode-lossless',dual,fast,'--from','0.08','--to','0.28','--streams','0,1','--seek')
        assert pixels(fast)==pixels(full)==pixels(dual,'trim=start=0.08:end=0.28')
        assert hashes(fast,'v:1')==hashes(full,'v:1')
        assert stats['seek_used'] and stats['copied_packets']>0 and stats['decoded_frames']<=baseline['decoded_frames']
        record('lossless seek keeps secondary non-reordered video sample-exact',stats=stats,baseline=baseline)
        full.unlink();fast.unlink()
        dual_h264=w/'dual-h264.mp4'
        ff('-f','lavfi','-i','testsrc2=size=128x72:rate=25','-f','lavfi','-i','testsrc2=size=96x54:rate=25','-t','4','-map','0:v','-map','1:v','-c:v','libx264','-bf','2','-g','25','-keyint_min','25','-sc_threshold','0','-pix_fmt','yuv420p',dual_h264)
        out=w/'dual-h264-idr.mkv';stats=native('transcode-lossless',dual_h264,out,'--from','1','--to','2','--streams','0,1')
        assert pixels(out)==pixels(dual_h264,'trim=start=1:end=2')
        ref=w/'dual-h264-sec.mp4';native('trim',dual_h264,ref,'--from','1','--to','2','--streams','1')
        assert hashes(out,'v:1')==hashes(ref,'v:0') and stats['copied_packets']>0
        record('lossless interval stream-copies H.264 secondary on closed-GOP IDR bounds',stats=stats)
        out=w/'dual-h264-mid.mkv';stats=native('transcode-lossless',dual_h264,out,'--from','1.08','--to','2.04','--streams','0,1')
        assert pixels(out)==pixels(dual_h264,'trim=start=1.08:end=2.04')
        ref=w/'dual-h264-mid-sec.mp4';native('trim',dual_h264,ref,'--from','1.08','--to','2.04','--streams','1')
        assert hashes(out,'v:1')==hashes(ref,'v:0') and stats['copied_packets']>0
        record('lossless interval stream-copies H.264 secondary with mid-GOP pre/post-roll',stats=stats)
        full=w/'dual-h264-full.mkv';fast=w/'dual-h264-seek.mkv'
        baseline=native('transcode-lossless',dual_h264,full,'--from','1','--to','2','--streams','0,1')
        stats=native('transcode-lossless',dual_h264,fast,'--from','1','--to','2','--streams','0,1','--seek')
        assert pixels(fast)==pixels(full)==pixels(dual_h264,'trim=start=1:end=2')
        assert hashes(fast,'v:1')==hashes(full,'v:1')
        assert stats['seek_used'] and stats['copied_packets']>0 and stats['decoded_frames']<=baseline['decoded_frames']
        record('lossless seek keeps H.264 secondary closed-GOP stream-copy',stats=stats,baseline=baseline)
        full.unlink();fast.unlink()
        plan=native('plan',source,'--crop','2:2:64:48')
        assert plan['command']=='transcode-lossless' and any(s['action']=='decode' for s in plan['steps']) and any(s['action']=='encode' for s in plan['steps'])
        assert plan['graph']=='crop=64:48:2:2:exact=1'
        record('plan explains decode/filter/encode without executing',plan=plan)
        plan=native('plan',source)
        assert plan['command']=='remux' and plan['steps'][0]['action']=='copy' and plan.get('graph') in (None,[])
        record('plan explains remux stream-copy',plan=plan)
        plan=native('plan',source,'--scale','64:36','--pix-fmt','yuv422p')
        assert plan['graph']=='scale=64:36:flags=neighbor,format=yuv422p'
        assert any('fused' in s['detail'] for s in plan['steps'] if s['action']=='materialize')
        record('plan explains fused scale+format graph',plan=plan)
        plan=native('plan',av_aac,'--from','0.125','--to','0.525')
        assert any(s['disposition']=='decode_pcm' for s in plan['streams'] if s['media_type']=='audio')
        assert any(s['action']=='decode' and 'PCM' in s['detail'] for s in plan['steps'])
        record('plan explains AAC interval decode to PCM',plan=plan)
        plan=native('plan','trim',source,'--from','1','--to','2')
        assert plan['command']=='trim' and plan['inputs']==[str(source)] and any(s['action']=='interval' for s in plan['steps'])
        record('plan explains trim stream-copy',plan=plan)
        plan=native('plan','concat',source,source)
        assert plan['command']=='concat' and len(plan['inputs'])==2 and any(s['action']=='concat' for s in plan['steps'])
        record('plan explains concat stream-copy',plan=plan)
        full=w/'av-aac-full.mkv';fast=w/'av-aac-seek.mkv'
        baseline=native('transcode-lossless',av_aac,full,'--from','0.125','--to','0.525')
        stats=native('transcode-lossless',av_aac,fast,'--from','0.125','--to','0.525','--seek')
        assert pixels(fast)==pixels(full)==pixels(av_aac,'trim=start=0.125:end=0.525')
        assert audio(fast)==audio(full)==ff('-i',av_aac,'-af','atrim=start_sample=6000:end_sample=25200,asetpts=PTS-STARTPTS','-c:a','pcm_s16le','-f','s16le','-').stdout
        assert stats['seek_used'] and stats['trimmed_audio_sample_frames']==19200 and stats['decoded_frames']<=baseline['decoded_frames']
        record('AAC seek keeps sample-exact lossless interval',stats=stats,baseline=baseline)
        full.unlink();fast.unlink()
        for codec,raw,bytes_per_sample in [('pcm_s24le','s24le',3),('pcm_s32le','s32le',4),('pcm_f32le','f32le',4)]:
            src=w/f'{codec}.mkv';ff('-i',av,'-c:v','copy','-ac','2','-af','asetnsamples=n=1920:p=0','-c:a',codec,src)
            out=w/f'{codec}-trim.mkv';stats=native('transcode-lossless',src,out,'--from','0.125','--to','0.525')
            original=ff('-i',src,'-map','0:a:0','-c:a',codec,'-f',raw,'-').stdout
            actual=ff('-i',out,'-map','0:a:0','-c:a',codec,'-f',raw,'-').stdout
            assert actual==original[6000*2*bytes_per_sample:25200*2*bytes_per_sample]
            assert pixels(out)==pixels(src,'trim=start=0.125:end=0.525')
            record(f'packed stereo {codec} sample-exact interval',stats=stats)
        out=w/'audio.wav';stats=native('remux',av,out,'--streams','1');assert audio(out)==audio(av);record('extract audio stream to WAV',stats=stats)
        wav=w/'audio.wav';out=w/'audio-trim.wav';stats=native('trim-pcm',wav,out,'--from','0.125','--to','0.525')
        assert audio(out)==audio(wav)[6000*2:25200*2]
        assert stats['sample_frames']==19200 and stats['payload_bytes']==38400 and stats['fvid_payload_copies']==0
        record('standalone WAV PCM slicing',stats=stats)
        out=w/'selected-trim.wav';stats=native('trim-pcm',av,out,'--streams','1','--from','0.125','--to','0.525')
        assert audio(out)==audio(av)[6000*2:25200*2]
        record('extract and trim PCM from video container',stats=stats)
        plan=native('plan','trim-pcm',wav,'--from','0.125','--to','0.525')
        assert plan['command']=='trim-pcm' and plan.get('graph') in (None,[]) and any(s['action']=='interval' for s in plan['steps'])
        assert any(s['disposition']=='trim_pcm' for s in plan['streams']) and any(s['action']=='trim' and 'packet-view' in s['detail'] for s in plan['steps'])
        assert any('atrim' in n for n in plan['notes'])
        record('plan trim-pcm subcommand explains packet-view slice',plan=plan)
        out=w/'av-trim.mkv';stats=native('trim',av,out,'--from','1','--to','2');assert pixels(out)==pixels(av)[25*128*72*3//2:];assert audio(out)==audio(av)[48000*2:];record('strict video+audio aligned trim',stats=stats)
        out=w/'av-concat.mkv';stats=native('concat',out,av,av);assert pixels(out)==pixels(av)*2;assert audio(out)==audio(av)*2;record('strict video+audio concat',stats=stats)
        high=w/'high.mkv';ff('-f','lavfi','-i','testsrc2=size=128x72:rate=25','-t','0.4','-pix_fmt','yuv420p10le','-c:v','ffv1','-level','3',high)
        out=w/'high-crop.mkv';stats=native('crop-lossless',high,out,'--crop','2:2:64:48');assert pixels(out,pixfmt='yuv420p10le')==pixels(high,'crop=64:48:2:2:exact=1','yuv420p10le');record('10-bit crop preserves samples',stats=stats)
        high12=w/'high12.mkv';ff('-f','lavfi','-i','testsrc2=size=128x72:rate=25','-t','0.4','-pix_fmt','yuv420p12le','-c:v','ffv1','-level','3',high12)
        out=w/'high12-crop.mkv';stats=native('crop-lossless',high12,out,'--crop','2:2:64:48');assert pixels(out,pixfmt='yuv420p12le')==pixels(high12,'crop=64:48:2:2:exact=1','yuv420p12le');record('12-bit crop preserves samples',stats=stats)
        for name,src,fmt,flags,reference in [
            ('identity lossless B-frame transcode',bframes,'yuv420p',[],None),
            ('vertical flip with copied PCM',av,'yuv420p',['--vflip'],'vflip'),
            ('crop then vertical flip 10-bit',high,'yuv420p10le',['--crop','2:2:64:48','--vflip'],'crop=64:48:2:2:exact=1,vflip'),
            ('neighbor scale after crop',source,'yuv420p',['--crop','2:2:64:48','--scale','32:24'],'crop=64:48:2:2:exact=1,scale=32:24:flags=neighbor'),
            ('neighbor scale full frame',source,'yuv420p',['--scale','64:36'],'scale=64:36:flags=neighbor'),
            ('format yuv422p',source,'yuv422p',['--pix-fmt','yuv422p'],'format=yuv422p'),
            ('scale then format yuv422p',source,'yuv422p',['--scale','64:36','--pix-fmt','yuv422p'],'scale=64:36:flags=neighbor,format=yuv422p'),
            ('transpose clock',source,'yuv420p',['--transpose','clock'],'transpose=clock'),
            ('transpose cclock after crop',source,'yuv420p',['--crop','2:2:64:48','--transpose','cclock'],'crop=64:48:2:2:exact=1,transpose=cclock'),
            ('transpose clock_flip',source,'yuv420p',['--transpose','clock_flip'],'transpose=clock_flip'),
            ('pad black canvas',source,'yuv420p',['--pad','192:108:32:18'],'pad=192:108:32:18:black'),
            ('rotate 45 degrees',source,'yuv420p',['--rotate','45'],'rotate=a=45*PI/180:ow=141:oh=141:c=black'),
        ]:
            out=w/'transform.mkv';stats=native('transcode-lossless',src,out,*flags)
            assert pixels(out,pixfmt=fmt)==pixels(src,reference,fmt)
            if src==av:assert audio(out)==audio(src)
            record(name,stats=stats);out.unlink()
        stats=native('decode',source,'--scale','64:36')
        assert stats['width']==64 and stats['height']==36 and stats['video_frames']==50
        record('decode reports neighbor scale geometry',stats=stats)
        stats=native('decode',source,'--transpose','clock')
        assert stats['width']==72 and stats['height']==128 and stats['video_frames']==50
        record('decode reports transpose geometry',stats=stats)
        stats=native('decode',source,'--pad','192:108:32:18')
        assert stats['width']==192 and stats['height']==108 and stats['video_frames']==50
        record('decode reports pad geometry',stats=stats)
        stats=native('decode',source,'--rotate','45')
        assert stats['width']==141 and stats['height']==141 and stats['video_frames']==50
        record('decode reports rotate geometry',stats=stats)
        stats=native('decode',source,'--pix-fmt','yuv422p')
        assert stats['pixel_format']=='yuv422p' and stats['video_frames']==50
        record('decode reports pix-fmt conversion',stats=stats)
        vfr=w/'vfr.mkv';ff('-f','lavfi','-i','testsrc2=size=128x72:rate=10','-frames:v','20','-vf','setpts=N+floor(N/2)','-fps_mode','vfr','-c:v','ffv1',vfr)
        out=w/'vfr-transcoded.mkv';stats=native('transcode-lossless',vfr,out)
        assert video_timeline(out)==video_timeline(vfr)
        assert pixels(out)==pixels(vfr)
        assert packets(out)==packets(vfr)
        assert stats['decoded_frames']==0 and stats['encoder']=='ffv1 (stream copy)'
        record('FFV1 identity planner preserves VFR packets and timeline',stats=stats)
        for fmt in ['yuv422p','yuv444p','yuva444p','gbrp10le','gray16le']:
            src=w/f'{fmt}.mkv';ff('-f','lavfi','-i','testsrc2=size=128x72:rate=25','-t','0.2','-pix_fmt',fmt,'-c:v','ffv1','-level','3',src)
            out=w/f'{fmt}-flip.mkv';stats=native('transcode-lossless',src,out,'--vflip')
            assert pixels(out,pixfmt=fmt)==pixels(src,'vflip',fmt)
            record(f'vertical flip preserves {fmt} planes',stats=stats)
        for fmt in ['yuv420p','yuv422p','yuv420p10le']:
            src=w/f'odd-{fmt}.mkv';ff('-f','lavfi','-i','testsrc=size=129x73:rate=25','-t','0.2','-pix_fmt',fmt,'-c:v','ffv1','-level','3',src)
            for flags,reference in [([],None),(['--vflip'],'vflip'),(['--hflip'],'hflip'),(['--crop','2:2:65:49','--vflip'],'crop=65:49:2:2:exact=1,vflip')]:
                out=w/'odd-output.mkv';stats=native('transcode-lossless',src,out,*flags)
                assert pixels(out,pixfmt=fmt)==pixels(src,reference,fmt)
                record(f'odd geometry {fmt} {flags}',stats=stats);out.unlink()
        for src,fmt in [(bframes,'yuv420p'),(high,'yuv420p10le'),(w/'yuva444p.mkv','yuva444p'),(w/'gbrp10le.mkv','gbrp10le')]:
            out=w/'horizontal.mkv';stats=native('transcode-lossless',src,out,'--crop','2:2:64:48','--hflip','--vflip')
            assert pixels(out,pixfmt=fmt)==pixels(src,'crop=64:48:2:2:exact=1,hflip,vflip',fmt)
            record(f'horizontal crop and vertical flip {src.name}',stats=stats);out.unlink()
        for encoder,extension,options in [
            ('libx264','mp4',['--encoder-option','crf=0','--encoder-option','preset=fast']),
            ('libvpx-vp9','webm',['--encoder-option','lossless=1','--encoder-option','deadline=good','--encoder-option','cpu-used=4']),
            ('libx265','mp4',['--encoder-option','x265-params=lossless=1:log-level=error','--encoder-option','preset=ultrafast']),
        ]:
            out=w/f'explicit-{encoder}.{extension}';stats=native('transcode',source,out,'--encoder',encoder,*options,'--crop','2:2:64:48','--hflip')
            assert pixels(out)==pixels(source,'crop=64:48:2:2:exact=1,hflip')
            assert stats['encoder']==encoder and stats['video_frames']==50
            record(f'explicit {encoder} lossless mode with crop/hflip',stats=stats)
        out=w/'lossy.mp4';stats=native('transcode',source,out,'--encoder','libx264','--encoder-option','crf=28')
        info=native('probe',out);assert info['streams'][0]['codec']=='h264'
        assert len(pixels(out))==len(pixels(source)) and pixels(out)!=pixels(source)
        record('explicit lossy H264 preserves frame count and geometry',stats=stats)
        out=w/'lossy-det.mp4';stats=native('transcode',source,out,'--encoder','libx264','--encoder-option','crf=28','--encoder-option','preset=veryfast','--encoder-option','threads=1','--encoder-option','x264-params=threads=1:sliced-threads=0')
        ref=w/'lossy-det-ff.mp4';ff('-i',source,'-c:v','libx264','-crf','28','-preset','veryfast','-threads','1','-x264-params','threads=1:sliced-threads=0','-an',ref)
        assert pixels(out)==pixels(ref) and stats['encoder']=='libx264'
        record('lossy H264 CRF/preset matches FFmpeg with deterministic threads',stats=stats)
        hevc=w/'hevc.mp4';ff('-f','lavfi','-i','testsrc2=size=128x72:rate=25','-t','2','-pix_fmt','yuv420p','-c:v','libx265','-preset','ultrafast','-x265-params','log-level=error',hevc)
        info=native('probe',hevc);assert info['streams'][0]['codec']=='hevc' and info['streams'][0]['profile']
        record('rich probe HEVC profile/timeline',info=info)
        stats=native('decode',hevc);assert stats['video_frames']==50 and stats['width']==128
        record('HEVC software decode frame count',stats=stats)
        out=w/'hevc-crop.mkv';stats=native('crop-lossless',hevc,out,'--crop','2:2:64:48')
        assert pixels(out)==pixels(hevc,'crop=64:48:2:2:exact=1')
        record('HEVC decode -> borrowed crop -> FFV1',stats=stats)
        out=w/'hevc-remux.mkv';stats=native('remux',hevc,out);assert hashes(hevc)==hashes(out)
        record('HEVC remux preserves compressed packets',stats=stats)
        av1=w/'av1.mp4';ff('-f','lavfi','-i','testsrc2=size=128x72:rate=25','-t','0.4','-pix_fmt','yuv420p','-c:v','libsvtav1','-preset','12','-crf','30',av1)
        info=native('probe',av1);assert info['streams'][0]['codec']=='av1'
        out=w/'av1-crop.mkv';stats=native('crop-lossless',av1,out,'--crop','2:2:64:48')
        assert pixels(out)==pixels(av1,'crop=64:48:2:2:exact=1')
        record('AV1 decode -> borrowed crop -> FFV1',stats=stats)
        prores=w/'prores.mov';ff('-f','lavfi','-i','testsrc2=size=128x72:rate=25','-t','0.4','-pix_fmt','yuv422p10le','-c:v','prores_ks','-profile:v','0',prores)
        info=native('probe',prores);assert 'prores' in info['streams'][0]['codec']
        out=w/'prores-crop.mkv';stats=native('crop-lossless',prores,out,'--crop','2:2:64:48')
        assert pixels(out,pixfmt='yuv422p10le')==pixels(prores,'crop=64:48:2:2:exact=1','yuv422p10le')
        record('ProRes 422 decode -> borrowed crop -> FFV1',stats=stats)
        aac=w/'aac.mp4';ff('-f','lavfi','-i','testsrc2=size=128x72:rate=25','-f','lavfi','-i','sine=frequency=733:sample_rate=48000','-t','2','-c:v','libx264','-c:a','aac',aac)
        out=w/'aac-remux.mp4';stats=native('remux',aac,out);assert hashes(aac,'a:0')==hashes(out,'a:0');assert audio(aac)==audio(out);record('AAC MP4 remux preserves decoded samples',stats=stats)
        out=w/'aac-crop.mkv';stats=native('crop-lossless',aac,out,'--crop','2:2:64:48');assert audio(aac)==audio(out);assert hashes(aac,'a:0')==hashes(out,'a:0');record('AAC crop preserves decoded samples',stats=stats)
        out=w/'aac-decoded.wav';stats=native('decode-audio',aac,out,'--streams','1')
        expected=ff('-i',aac,'-map','0:a:0','-c:a','pcm_f32le','-f','f32le','-').stdout
        actual=ff('-i',out,'-c:a','pcm_f32le','-f','f32le','-').stdout
        assert actual==expected and stats['sample_format']=='flt'
        assert stats['channels']==1 and stats['planar_interleave_bytes']==0
        record('AAC decode to float PCM preserves every decoded sample',stats=stats)
        out=w/'aac-resampled.wav';stats=native('decode-audio',aac,out,'--streams','1','--rate','44100')
        expected=ff('-i',aac,'-map','0:a:0','-ar','44100','-c:a','pcm_f32le','-f','f32le','-').stdout
        actual=ff('-i',out,'-c:a','pcm_f32le','-f','f32le','-').stdout
        assert actual==expected and stats['sample_rate']==44100 and stats['sample_format']=='flt'
        record('AAC decode resamples to 44100 with sample-exact PCM',stats=stats)
        out=w/'aac-volume.wav';stats=native('decode-audio',aac,out,'--streams','1','--volume','0.5')
        expected=ff('-i',aac,'-map','0:a:0','-af','volume=0.5','-c:a','pcm_f32le','-f','f32le','-').stdout
        actual=ff('-i',out,'-c:a','pcm_f32le','-f','f32le','-').stdout
        assert actual==expected and stats['sample_format']=='flt'
        record('AAC decode applies linear volume with sample-exact PCM',stats=stats)
        out=w/'aac-interval.wav';stats=native('decode-audio',aac,out,'--streams','1','--from','1.123','--to','1.789')
        expected=ff('-i',aac,'-map','0:a:0','-af','atrim=start_sample=53904:end_sample=85872,asetpts=PTS-STARTPTS','-c:a','pcm_f32le','-f','f32le','-').stdout
        actual=ff('-i',out,'-c:a','pcm_f32le','-f','f32le','-').stdout
        assert actual==expected and stats['sample_frames']==31968
        record('AAC interval cuts inside decoded frames with exact float samples',stats=stats)
        stereo=w/'stereo-aac.m4a';ff('-f','lavfi','-i','aevalsrc=sin(2*PI*997*t)|sin(2*PI*1733*t):s=48000','-t','1','-c:a','aac',stereo)
        out=w/'stereo-decoded.wav';stats=native('decode-audio',stereo,out)
        expected=ff('-i',stereo,'-c:a','pcm_f32le','-f','f32le','-').stdout
        actual=ff('-i',out,'-c:a','pcm_f32le','-f','f32le','-').stdout
        assert actual==expected and stats['planar_interleave_bytes']==len(actual)
        record('stereo AAC distinct channels preserve bits through pooled SIMD interleave',stats=stats)
        out=w/'stereo-mono.wav';stats=native('decode-audio',stereo,out,'--channels','1')
        expected=ff('-i',stereo,'-ac','1','-c:a','pcm_f32le','-f','f32le','-').stdout
        actual=ff('-i',out,'-c:a','pcm_f32le','-f','f32le','-').stdout
        assert actual==expected and stats['channels']==1 and stats['sample_format']=='flt'
        record('stereo AAC rematrixes to mono with sample-exact PCM',stats=stats)
        out=w/'stereo-mono-rate.wav';stats=native('decode-audio',stereo,out,'--channels','1','--rate','44100')
        expected=ff('-i',stereo,'-ac','1','-ar','44100','-c:a','pcm_f32le','-f','f32le','-').stdout
        actual=ff('-i',out,'-c:a','pcm_f32le','-f','f32le','-').stdout
        assert actual==expected and stats['channels']==1 and stats['sample_rate']==44100
        record('stereo AAC rematrix+resample matches FFmpeg -ac/-ar',stats=stats)
        plan=native('plan','decode-audio',aac,'--streams','1')
        assert plan['command']=='decode-audio' and any(s['action']=='decode' for s in plan['steps']) and any(s['action']=='encode' for s in plan['steps'])
        record('plan decode-audio subcommand explains decode+WAV',plan=plan)
        plan=native('plan','decode-audio',stereo,'--channels','1','--rate','44100','--volume','0.5')
        assert plan['command']=='decode-audio' and 'aresample=44100' in plan['graph'] and 'aformat=channel_layouts=mono' in plan['graph'] and 'volume=0.5' in plan['graph']
        record('plan decode-audio subcommand explains -af chain',plan=plan)
        mix_a=w/'mix-a.m4a';ff('-f','lavfi','-i','sine=frequency=440:sample_rate=48000','-t','0.4','-ac','2','-c:a','aac','-b:a','192k',mix_a)
        mix_b=w/'mix-b.m4a';ff('-f','lavfi','-i','sine=frequency=880:sample_rate=48000','-t','0.4','-ac','2','-c:a','aac','-b:a','192k',mix_b)
        ff('-f','lavfi','-i','sine=frequency=660:sample_rate=44100','-t','0.4','-ac','2','-c:a','aac','-b:a','192k',w/'mix-44k.m4a')
        out=w/'mix-norm.wav';stats=native('mix-audio',out,mix_a,mix_b)
        expected=ff('-i',mix_a,'-i',mix_b,'-filter_complex','[0:a][1:a]amix=inputs=2:duration=shortest:dropout_transition=0:normalize=1','-c:a','pcm_f32le','-f','f32le','-').stdout
        actual=ff('-i',out,'-c:a','pcm_f32le','-f','f32le','-').stdout
        assert actual==expected and stats['normalize'] is True and stats['sample_frames']>0
        record('mix-audio matches FFmpeg amix normalize=1 sample-exact',stats=stats)
        out=w/'mix-sum.wav';stats=native('mix-audio',out,mix_a,mix_b,'--no-normalize')
        expected=ff('-i',mix_a,'-i',mix_b,'-filter_complex','[0:a][1:a]amix=inputs=2:duration=shortest:dropout_transition=0:normalize=0','-c:a','pcm_f32le','-f','f32le','-').stdout
        actual=ff('-i',out,'-c:a','pcm_f32le','-f','f32le','-').stdout
        assert actual==expected and stats['normalize'] is False
        record('mix-audio matches FFmpeg amix normalize=0 sample-exact',stats=stats)
        mix_c=w/'mix-c.m4a';ff('-f','lavfi','-i','sine=frequency=660:sample_rate=48000','-t','0.4','-ac','2','-c:a','aac','-b:a','192k',mix_c)
        out=w/'mix-3.wav';stats=native('mix-audio',out,mix_a,mix_b,mix_c)
        expected=ff('-i',mix_a,'-i',mix_b,'-i',mix_c,'-filter_complex','[0:a][1:a][2:a]amix=inputs=3:duration=shortest:dropout_transition=0:normalize=1,aformat=sample_fmts=flt','-c:a','pcm_f32le','-f','f32le','-').stdout
        actual=ff('-i',out,'-c:a','pcm_f32le','-f','f32le','-').stdout
        assert actual==expected and stats['inputs']==3 and stats['weights']==[1.0,1.0,1.0]
        record('mix-audio matches FFmpeg amix inputs=3 normalize=1 sample-exact',stats=stats)
        out=w/'mix-w0.wav';stats=native('mix-audio',out,mix_a,mix_b,mix_c,'--weights','1,2,0.5','--no-normalize')
        expected=ff('-i',mix_a,'-i',mix_b,'-i',mix_c,'-filter_complex','[0:a][1:a][2:a]amix=inputs=3:duration=shortest:dropout_transition=0:weights=1 2 0.5:normalize=0,aformat=sample_fmts=flt','-c:a','pcm_f32le','-f','f32le','-').stdout
        actual=ff('-i',out,'-c:a','pcm_f32le','-f','f32le','-').stdout
        assert actual==expected and stats['normalize'] is False and stats['weights']==[1.0,2.0,0.5]
        record('mix-audio matches FFmpeg amix weights normalize=0 sample-exact',stats=stats)
        out=w/'mix-w1.wav';stats=native('mix-audio',out,mix_a,mix_b,mix_c,'--weights','1,2,0.5')
        expected=ff('-i',mix_a,'-i',mix_b,'-i',mix_c,'-filter_complex','[0:a][1:a][2:a]amix=inputs=3:duration=shortest:dropout_transition=0:weights=1 2 0.5:normalize=1,aformat=sample_fmts=flt','-c:a','pcm_f32le','-f','f32le','-').stdout
        actual=ff('-i',out,'-c:a','pcm_f32le','-f','f32le','-').stdout
        assert actual==expected and stats['normalize'] is True
        record('mix-audio matches FFmpeg amix weights normalize=1 sample-exact',stats=stats)
        merge_l=w/'merge-l.m4a';ff('-f','lavfi','-i','sine=frequency=440:sample_rate=48000','-t','0.4','-ac','1','-c:a','aac','-b:a','192k',merge_l)
        merge_r=w/'merge-r.m4a';ff('-f','lavfi','-i','sine=frequency=880:sample_rate=48000','-t','0.4','-ac','1','-c:a','aac','-b:a','192k',merge_r)
        out=w/'amerge-stereo.wav';stats=native('merge-audio',out,merge_l,merge_r)
        expected=ff('-i',merge_l,'-i',merge_r,'-filter_complex','[0:a][1:a]amerge=inputs=2','-c:a','pcm_f32le','-f','f32le','-').stdout
        actual=ff('-i',out,'-c:a','pcm_f32le','-f','f32le','-').stdout
        assert actual==expected and stats['channels']==2 and stats['inputs']==2 and stats['sample_frames']>0
        record('merge-audio matches FFmpeg amerge mono+mono sample-exact',stats=stats)
        merge_st=w/'merge-st.m4a';ff('-f','lavfi','-i','aevalsrc=sin(2*PI*220*t)|sin(2*PI*330*t):s=48000','-t','0.35','-c:a','aac','-b:a','192k',merge_st)
        out=w/'amerge-3ch.wav';stats=native('merge-audio',out,merge_st,merge_l)
        expected=ff('-i',merge_st,'-i',merge_l,'-filter_complex','[0:a][1:a]amerge=inputs=2','-c:a','pcm_f32le','-f','f32le','-').stdout
        actual=ff('-i',out,'-c:a','pcm_f32le','-f','f32le','-').stdout
        assert actual==expected and stats['channels']==3
        record('merge-audio matches FFmpeg amerge stereo+mono sample-exact',stats=stats)
        plan=native('plan','mix-audio',mix_a,mix_b,'--no-normalize','--weights','1,2')
        assert plan['command']=='mix-audio' and 'amix=inputs=2' in plan['graph'] and 'normalize=0' in plan['graph'] and 'weights=1 2' in plan['graph']
        record('plan mix-audio subcommand explains amix graph',plan=plan)
        plan=native('plan','merge-audio',merge_l,merge_r)
        assert plan['command']=='merge-audio' and plan['graph']=='amerge=inputs=2'
        record('plan merge-audio subcommand explains amerge',plan=plan)
        for codec,extension,pcm,raw in [('libmp3lame','mp3','pcm_f32le','f32le'),('flac','flac','pcm_s16le','s16le')]:
            src=w/f'audio.{extension}';ff('-i',av,'-map','0:a:0','-c:a',codec,src)
            out=w/f'decoded-{extension}.wav';stats=native('decode-audio',src,out)
            expected=ff('-i',src,'-c:a',pcm,'-f',raw,'-').stdout
            actual=ff('-i',out,'-c:a',pcm,'-f',raw,'-').stdout
            assert actual==expected
            record(f'{extension} decode preserves samples and codec delay handling',stats=stats)
            out=w/f'interval-{extension}.wav';stats=native('decode-audio',src,out,'--from','1.123','--to','1.789')
            expected=ff('-i',src,'-af','atrim=start_sample=53904:end_sample=85872,asetpts=PTS-STARTPTS','-c:a',pcm,'-f',raw,'-').stdout
            actual=ff('-i',out,'-c:a',pcm,'-f',raw,'-').stdout
            assert actual==expected and stats['sample_frames']==31968
            record(f'{extension} interval preserves exact decoded samples',stats=stats)
        metadata=w/'chapters.txt';metadata.write_text(';FFMETADATA1\ntitle=Fixture\n[CHAPTER]\nTIMEBASE=1/1000\nSTART=0\nEND=1000\ntitle=First\n')
        chaptered=w/'chaptered.mkv';ff('-i',source,'-i',metadata,'-map','0','-map_metadata','1','-map_chapters','1','-c','copy',chaptered)
        info=native('probe',chaptered)
        assert info['metadata']['title']=='Fixture'
        assert len(info['chapters'])==1 and info['chapters'][0]['metadata']['title']=='First'
        chapter=info['chapters'][0]
        assert chapter['start']==0 and chapter['end']*chapter['time_base'][0]==chapter['time_base'][1]
        record('probe exposes container metadata and chapter timeline',info=info)
        for mode,flags in [('remux',[]),('crop-lossless',['--crop','2:2:64:48'])]:
            out=w/f'chapters-{mode}.mkv';native(mode,chaptered,out,*flags)
            chapters=json.loads(run(['ffprobe','-v','error','-show_chapters','-of','json',out]).stdout)['chapters']
            assert len(chapters)==1 and float(chapters[0]['start_time'])==0 and float(chapters[0]['end_time'])==1 and chapters[0]['tags']['title']=='First'
            record(f'{mode} preserves chapters')
        out=w/'meta-set.mkv';stats=native('remux',chaptered,out,'--metadata','title=Renamed','--metadata','artist=Fvid')
        info=native('probe',out)
        meta={k.lower():v for k,v in info['metadata'].items()}
        assert meta['title']=='Renamed' and meta['artist']=='Fvid'
        assert info['chapters'][0]['metadata']['title']=='First'
        assert hashes(out)==hashes(chaptered)
        reference=w/'meta-ffmpeg.mkv';ff('-i',chaptered,'-c','copy','-metadata','title=Renamed','-metadata','artist=Fvid',reference)
        ref={k.lower():v for k,v in native('probe',reference)['metadata'].items()}
        assert ref['title']=='Renamed' and ref['artist']=='Fvid'
        record('remux sets container metadata without rewriting packets',stats=stats)
        out=w/'meta-delete.mkv';stats=native('remux',chaptered,out,'--metadata-delete','title')
        assert 'title' not in native('probe',out)['metadata']
        assert hashes(out)==hashes(chaptered)
        record('remux deletes container metadata keys',stats=stats)
        subtitles=w/'captions.srt'
        subtitles.write_text('1\n00:00:00,125 --> 00:00:00,625\nalpha\n\n2\n00:00:01,125 --> 00:00:01,875\nbeta\n')
        out=w/'standalone-subtitles.mkv';native('remux',subtitles,out)
        assert packets(out,'s:0')==packets(subtitles,'s:0')
        record('standalone SRT remux preserves text packets and timestamps')
        out=w/'ass-fvid.mkv';stats=native('convert-subtitles',subtitles,out,'--codec','ass')
        assert stats['encoder']=='ass' and stats['cues']==2
        info=native('probe',out);assert info['streams'][0]['codec']=='ass'
        reference=w/'ass-ffmpeg.mkv';ff('-i',subtitles,'-c:s','ass',reference)
        def srt_text(path):
            return ff('-i',path,'-map','0:s:0','-c:s','srt','-f','srt','-').stdout.replace(b'\r\n',b'\n').strip()
        assert srt_text(out)==srt_text(reference)==srt_text(subtitles)
        record('standalone SRT to ASS cue timing and text equality',stats=stats)
        burn_subs=w/'burn.srt'
        burn_subs.write_text('1\n00:00:00,000 --> 00:00:01,000\nHELLO\n')
        out=w/'burned.mkv'
        stats=native('burn-subtitles',source,out,'--subs',str(burn_subs))
        burn_path=str(burn_subs.resolve()).replace('\\','/')
        burn_vf="subtitles='"+burn_path.replace(':','\\:')+"'"
        assert pixels(out)==pixels(source,burn_vf)
        record('burn-subtitles matches FFmpeg subtitles= pixel-exact',stats=stats)
        ov=w/'ov.mp4';ff('-f','lavfi','-i','testsrc2=size=32x24:rate=25','-t','0.4','-pix_fmt','yuv420p','-c:v','libx264','-preset','veryfast','-an',ov)
        out=w/'overlay.mkv';stats=native('overlay',source,ov,out,'--overlay-x','8','--overlay-y','8')
        vf=movie_overlay_vf(ov,8,8)
        assert pixels(out)==pixels(source,vf)
        record('overlay matches FFmpeg movie=+overlay= pixel-exact',stats=stats)
        ov_a=w/'ov-alpha.mkv';ff('-f','lavfi','-i','color=c=red@0.5:s=32x24:r=25,format=yuva420p','-t','0.4','-c:v','ffv1','-level','3',ov_a)
        out=w/'overlay-alpha.mkv';stats=native('overlay',source,ov_a,out,'--overlay-x','8','--overlay-y','8')
        assert pixels(out)==pixels(source,movie_overlay_vf(ov_a,8,8))
        record('overlay with alpha blends pixel-exact vs FFmpeg',stats=stats)
        plan=native('plan',source,'--overlay',str(ov),'--overlay-x','8','--overlay-y','8')
        assert plan['graph'] and 'overlay=8:8' in plan['graph']
        record('plan explains overlay graph',plan=plan)
        plan=native('plan','overlay',source,'--overlay',str(ov),'--overlay-x','8','--overlay-y','8')
        assert plan['command']=='overlay' and plan['graph'] and 'overlay=8:8' in plan['graph']
        record('plan overlay subcommand explains movie+overlay',plan=plan)
        xa=w/'xfade-a.mkv';ff('-f','lavfi','-i','testsrc2=size=128x72:rate=25','-t','0.8','-an','-pix_fmt','yuv420p','-c:v','ffv1','-level','3',xa)
        xb=w/'xfade-b.mkv';ff('-f','lavfi','-i','color=c=blue:s=128x72:r=25','-t','0.8','-an','-pix_fmt','yuv420p','-c:v','ffv1','-level','3',xb)
        out=w/'xfade.mkv';stats=native('xfade',xa,xb,out,'--xfade-duration','0.2','--xfade-offset','0.4','--transition','fade')
        fc='[0:v][1:v]xfade=transition=fade:duration=0.2:offset=0.4,format=yuv420p'
        expected=ff('-i',xa,'-i',xb,'-filter_complex',fc,'-an','-fps_mode','passthrough','-pix_fmt','yuv420p','-f','rawvideo','-').stdout
        assert pixels(out)==expected
        record('xfade matches FFmpeg dual-input filter_complex pixel-exact',stats=stats)
        plan=native('plan','xfade',xa,xb,'--xfade-duration','0.2','--xfade-offset','0.4','--transition','fade')
        assert plan['command']=='xfade' and plan['graph']==fc
        record('plan xfade subcommand explains filter_complex',plan=plan)
        plan=native('plan','burn-subtitles',source,'--subs',str(burn_subs))
        assert plan['command']=='burn-subtitles' and plan['graph'] and plan['graph'].startswith('subtitles=')
        record('plan burn-subtitles subcommand explains subtitles=',plan=plan)
        result=subprocess.run([str(BINARY),'media','xfade',str(xa),str(w/'missing-xfade.mkv'),str(w/'xfade-miss.mkv'),'--xfade-duration','0.2'],capture_output=True,timeout=60)
        assert result.returncode and not (w/'xfade-miss.mkv').exists()
        record('reject missing xfade other',error=result.stderr.decode())
        result=subprocess.run([str(BINARY),'media','xfade',str(xa),str(xb),str(w/'xfade-bad.mkv'),'--xfade-duration','0.2','--transition','nope'],capture_output=True,timeout=60)
        assert result.returncode and not (w/'xfade-bad.mkv').exists()
        record('reject invalid xfade transition',error=result.stderr.decode())
        cs=w/'colorspace-src.mkv'
        ff('-f','lavfi','-i','testsrc2=size=128x72:rate=25','-t','0.4','-pix_fmt','yuv420p','-colorspace','bt470bg','-color_primaries','bt470bg','-color_trc','gamma28','-c:v','ffv1','-level','3',cs)
        out=w/'colorspace.mkv';cs_args='iall=bt470bg:all=bt709'
        stats=native('transcode-lossless',cs,out,'--colorspace',cs_args)
        assert pixels(out)==pixels(cs,f'colorspace={cs_args}')
        record('colorspace bt470bg→bt709 matches FFmpeg',stats=stats)
        plan=native('plan',cs,'--colorspace',cs_args)
        assert plan['graph']==f'colorspace={cs_args}'
        record('plan explains colorspace graph',plan=plan)
        hdr=w/'hdr-tonemap.mkv'
        ff('-f','lavfi','-i','testsrc2=size=128x72:rate=25','-t','0.4','-pix_fmt','yuv420p10le','-colorspace','bt2020nc','-color_range','tv','-c:v','ffv1','-level','3',hdr)
        out=w/'tonemap.mkv';tm_args='tonemap=hable'
        stats=native('transcode-lossless',hdr,out,'--tonemap',tm_args,'--pix-fmt','yuv420p')
        assert pixels(out)==pixels(hdr,f'tonemap={tm_args},format=yuv420p')
        record('tonemap hable+format matches FFmpeg',stats=stats)
        plan=native('plan',hdr,'--tonemap',tm_args,'--pix-fmt','yuv420p')
        assert plan['graph']==f'tonemap={tm_args},format=yuv420p'
        record('plan explains tonemap+format graph',plan=plan)
        zs=w/'zscale-src.mkv'
        ff('-f','lavfi','-i','testsrc2=size=128x72:rate=25','-t','0.4','-pix_fmt','yuv420p','-colorspace','bt470bg','-color_primaries','bt470bg','-color_trc','gamma28','-c:v','ffv1','-level','3',zs)
        out=w/'zscale.mkv';zs_args='matrixin=bt470bg:matrix=bt709:transferin=bt470bg:transfer=bt709:primariesin=bt470bg:primaries=bt709'
        stats=native('transcode-lossless',zs,out,'--zscale',zs_args,'--pix-fmt','yuv420p')
        assert pixels(out)==pixels(zs,f'zscale={zs_args},format=yuv420p')
        record('zscale bt470bg→bt709+format matches FFmpeg',stats=stats)
        plan=native('plan',zs,'--zscale',zs_args,'--pix-fmt','yuv420p')
        assert plan['graph']==f'zscale={zs_args},format=yuv420p'
        record('plan explains zscale+format graph',plan=plan)
        yd_args='mode=0'
        out=w/'yadif.mkv';stats=native('transcode-lossless',source,out,'--yadif',yd_args)
        assert pixels(out)==pixels(source,f'yadif={yd_args}')
        assert stats['video_frames']==50  # 2s@25fps progressive → mode=0 send_frame is 1:1
        record('yadif matches FFmpeg pixel-exact',stats=stats)
        plan=native('plan',source,'--yadif',yd_args)
        assert plan['graph']==f'yadif={yd_args}'
        record('plan explains yadif graph',plan=plan)
        bw_args='mode=0'
        out=w/'bwdif.mkv';stats=native('transcode-lossless',source,out,'--bwdif',bw_args)
        assert pixels(out)==pixels(source,f'bwdif={bw_args}')
        assert stats['video_frames']==50  # 2s@25fps progressive → mode=0 send_frame is 1:1
        record('bwdif matches FFmpeg pixel-exact',stats=stats)
        plan=native('plan',source,'--bwdif',bw_args)
        assert plan['graph']==f'bwdif={bw_args}'
        record('plan explains bwdif graph',plan=plan)
        w3_args='mode=0'
        out=w/'w3fdif.mkv';stats=native('transcode-lossless',source,out,'--w3fdif',w3_args)
        assert pixels(out)==pixels(source,f'w3fdif={w3_args}')
        assert stats['video_frames']==50  # 2s@25fps progressive → mode=0 send_frame is 1:1
        record('w3fdif matches FFmpeg pixel-exact',stats=stats)
        plan=native('plan',source,'--w3fdif',w3_args)
        assert plan['graph']==f'w3fdif={w3_args}'
        record('plan explains w3fdif graph',plan=plan)
        tb_args='all_mode=average'
        out=w/'tblend.mkv';stats=native('transcode-lossless',source,out,'--tblend',tb_args)
        assert pixels(out)==pixels(source,f'tblend={tb_args}')
        assert stats['video_frames']==49  # 2s@25fps → tblend emits N-1
        record('tblend average matches FFmpeg pixel-exact',stats=stats)
        plan=native('plan',source,'--tblend',tb_args)
        assert plan['graph']==f'tblend={tb_args}'
        record('plan explains tblend graph',plan=plan)
        mx_args='frames=3'
        out=w/'tmix.mkv';stats=native('transcode-lossless',source,out,'--tmix',mx_args)
        assert pixels(out)==pixels(source,f'tmix={mx_args}')
        assert stats['video_frames']==50  # 2s@25fps → tmix=frames=3 emits 50 (flush)
        record('tmix matches FFmpeg pixel-exact',stats=stats)
        plan=native('plan',source,'--tmix',mx_args)
        assert plan['graph']==f'tmix={mx_args}'
        record('plan explains tmix graph',plan=plan)
        dn_args='4:3:6:4.5'
        out=w/'hqdn3d.mkv';stats=native('transcode-lossless',source,out,'--hqdn3d',dn_args)
        assert pixels(out)==pixels(source,f'hqdn3d={dn_args}')
        record('hqdn3d matches FFmpeg pixel-exact',stats=stats)
        plan=native('plan',source,'--hqdn3d',dn_args)
        assert plan['graph']==f'hqdn3d={dn_args}'
        record('plan explains hqdn3d graph',plan=plan)
        out=w/'fps.mkv';stats=native('transcode-lossless',source,out,'--fps','12')
        assert pixels(out)==pixels(source,'fps=12')
        assert stats['video_frames']==24  # 2s@25fps → fps=12 emits 24
        record('fps CFR matches FFmpeg pixel-exact',stats=stats)
        plan=native('plan',source,'--fps','12')
        assert plan['graph']=='fps=12'
        record('plan explains fps graph',plan=plan)
        gb_args='sigma=1.5:steps=1'
        out=w/'gblur.mkv';stats=native('transcode-lossless',source,out,'--gblur',gb_args)
        assert pixels(out)==pixels(source,f'gblur={gb_args}')
        record('gblur matches FFmpeg pixel-exact',stats=stats)
        plan=native('plan',source,'--gblur',gb_args)
        assert plan['graph']==f'gblur={gb_args}'
        record('plan explains gblur graph',plan=plan)
        eq_args='brightness=0.06:contrast=1.2'
        out=w/'eq.mkv';stats=native('transcode-lossless',source,out,'--eq',eq_args)
        assert pixels(out)==pixels(source,f'eq={eq_args}')
        record('eq matches FFmpeg pixel-exact',stats=stats)
        plan=native('plan',source,'--eq',eq_args)
        assert plan['graph']==f'eq={eq_args}'
        record('plan explains eq graph',plan=plan)
        us_args='5:5:1.0:5:5:0.0'
        out=w/'unsharp.mkv';stats=native('transcode-lossless',source,out,'--unsharp',us_args)
        assert pixels(out)==pixels(source,f'unsharp={us_args}')
        record('unsharp matches FFmpeg pixel-exact',stats=stats)
        plan=native('plan',source,'--unsharp',us_args)
        assert plan['graph']==f'unsharp={us_args}'
        record('plan explains unsharp graph',plan=plan)
        hue_args='h=45:s=1.2'
        out=w/'hue.mkv';stats=native('transcode-lossless',source,out,'--hue',hue_args)
        assert pixels(out)==pixels(source,f'hue={hue_args}')
        record('hue matches FFmpeg pixel-exact',stats=stats)
        plan=native('plan',source,'--hue',hue_args)
        assert plan['graph']==f'hue={hue_args}'
        record('plan explains hue graph',plan=plan)
        bb_args='2:1'
        out=w/'boxblur.mkv';stats=native('transcode-lossless',source,out,'--boxblur',bb_args)
        assert pixels(out)==pixels(source,f'boxblur={bb_args}')
        record('boxblur matches FFmpeg pixel-exact',stats=stats)
        plan=native('plan',source,'--boxblur',bb_args)
        assert plan['graph']==f'boxblur={bb_args}'
        record('plan explains boxblur graph',plan=plan)
        ab_args='sizeX=5'
        out=w/'avgblur.mkv';stats=native('transcode-lossless',source,out,'--avgblur',ab_args)
        assert pixels(out)==pixels(source,f'avgblur={ab_args}')
        record('avgblur matches FFmpeg pixel-exact',stats=stats)
        plan=native('plan',source,'--avgblur',ab_args)
        assert plan['graph']==f'avgblur={ab_args}'
        record('plan explains avgblur graph',plan=plan)
        out=w/'negate.mkv';stats=native('transcode-lossless',source,out,'--negate','0')
        assert pixels(out)==pixels(source,'negate')
        record('negate matches FFmpeg pixel-exact',stats=stats)
        plan=native('plan',source,'--negate','0')
        assert plan['graph']=='negate'
        record('plan explains negate graph',plan=plan)
        ed_args='mode=colormix'
        out=w/'edgedetect.mkv';stats=native('transcode-lossless',source,out,'--edgedetect',ed_args)
        assert pixels(out)==pixels(source,f'edgedetect={ed_args}')
        record('edgedetect matches FFmpeg pixel-exact',stats=stats)
        plan=native('plan',source,'--edgedetect',ed_args)
        assert plan['graph']==f'edgedetect={ed_args}'
        record('plan explains edgedetect graph',plan=plan)
        out=w/'sobel.mkv';stats=native('transcode-lossless',source,out,'--sobel','')
        assert pixels(out)==pixels(source,'sobel')
        record('sobel matches FFmpeg pixel-exact',stats=stats)
        plan=native('plan',source,'--sobel','')
        assert plan['graph']=='sobel'
        record('plan explains sobel graph',plan=plan)
        out=w/'prewitt.mkv';stats=native('transcode-lossless',source,out,'--prewitt','')
        assert pixels(out)==pixels(source,'prewitt')
        record('prewitt matches FFmpeg pixel-exact',stats=stats)
        plan=native('plan',source,'--prewitt','')
        assert plan['graph']=='prewitt'
        record('plan explains prewitt graph',plan=plan)
        out=w/'roberts.mkv';stats=native('transcode-lossless',source,out,'--roberts','')
        assert pixels(out)==pixels(source,'roberts')
        record('roberts matches FFmpeg pixel-exact',stats=stats)
        plan=native('plan',source,'--roberts','')
        assert plan['graph']=='roberts'
        record('plan explains roberts graph',plan=plan)
        out=w/'kirsch.mkv';stats=native('transcode-lossless',source,out,'--kirsch','')
        assert pixels(out)==pixels(source,'kirsch')
        record('kirsch matches FFmpeg pixel-exact',stats=stats)
        plan=native('plan',source,'--kirsch','')
        assert plan['graph']=='kirsch'
        record('plan explains kirsch graph',plan=plan)
        out=w/'scharr.mkv';stats=native('transcode-lossless',source,out,'--scharr','')
        assert pixels(out)==pixels(source,'scharr')
        record('scharr matches FFmpeg pixel-exact',stats=stats)
        plan=native('plan',source,'--scharr','')
        assert plan['graph']=='scharr'
        record('plan explains scharr graph',plan=plan)
        ata_args='0a=0.02:0b=0.04'
        out=w/'atadenoise.mkv';stats=native('transcode-lossless',source,out,'--atadenoise',ata_args)
        assert pixels(out)==pixels(source,f'atadenoise={ata_args}')
        record('atadenoise matches FFmpeg pixel-exact',stats=stats)
        plan=native('plan',source,'--atadenoise',ata_args)
        assert plan['graph']==f'atadenoise={ata_args}'
        record('plan explains atadenoise graph',plan=plan)
        ow_args='depth=8:luma_strength=1.0:chroma_strength=1.0'
        out=w/'owdenoise.mkv';stats=native('transcode-lossless',source,out,'--owdenoise',ow_args)
        assert pixels(out)==pixels(source,f'owdenoise={ow_args}')
        record('owdenoise matches FFmpeg pixel-exact',stats=stats)
        plan=native('plan',source,'--owdenoise',ow_args)
        assert plan['graph']==f'owdenoise={ow_args}'
        record('plan explains owdenoise graph',plan=plan)
        vd_args='threshold=3'
        out=w/'vaguedenoiser.mkv';stats=native('transcode-lossless',source,out,'--vaguedenoiser',vd_args)
        assert pixels(out)==pixels(source,f'vaguedenoiser={vd_args}')
        record('vaguedenoiser matches FFmpeg pixel-exact',stats=stats)
        plan=native('plan',source,'--vaguedenoiser',vd_args)
        assert plan['graph']==f'vaguedenoiser={vd_args}'
        record('plan explains vaguedenoiser graph',plan=plan)
        nl_args='s=1.0'
        out=w/'nlmeans.mkv';stats=native('transcode-lossless',source,out,'--nlmeans',nl_args)
        assert pixels(out)==pixels(source,f'nlmeans={nl_args}')
        record('nlmeans matches FFmpeg pixel-exact',stats=stats)
        plan=native('plan',source,'--nlmeans',nl_args)
        assert plan['graph']==f'nlmeans={nl_args}'
        record('plan explains nlmeans graph',plan=plan)
        bm3d_args='sigma=3'
        out=w/'bm3d.mkv';stats=native('transcode-lossless',source,out,'--bm3d',bm3d_args)
        assert pixels(out)==pixels(source,f'bm3d={bm3d_args}')
        record('bm3d matches FFmpeg pixel-exact',stats=stats)
        plan=native('plan',source,'--bm3d',bm3d_args)
        assert plan['graph']==f'bm3d={bm3d_args}'
        record('plan explains bm3d graph',plan=plan)
        dctdnoiz_args='s=3'
        out=w/'dctdnoiz.mkv';stats=native('transcode-lossless',source,out,'--dctdnoiz',dctdnoiz_args)
        assert pixels(out)==pixels(source,f'dctdnoiz={dctdnoiz_args}')
        record('dctdnoiz matches FFmpeg pixel-exact',stats=stats)
        plan=native('plan',source,'--dctdnoiz',dctdnoiz_args)
        assert plan['graph']==f'dctdnoiz={dctdnoiz_args}'
        record('plan explains dctdnoiz graph',plan=plan)
        fftdnoiz_args='sigma=1:method=hard:block=128'
        out=w/'fftdnoiz.mkv';stats=native('transcode-lossless',source,out,'--fftdnoiz',fftdnoiz_args)
        assert pixels(out)==pixels(source,f'fftdnoiz={fftdnoiz_args}')
        record('fftdnoiz matches FFmpeg pixel-exact',stats=stats)
        plan=native('plan',source,'--fftdnoiz',fftdnoiz_args)
        assert plan['graph']==f'fftdnoiz={fftdnoiz_args}'
        record('plan explains fftdnoiz graph',plan=plan)
        sb_args='lr=1.5:ls=-0.5'
        out=w/'smartblur.mkv';stats=native('transcode-lossless',source,out,'--smartblur',sb_args)
        assert pixels(out)==pixels(source,f'smartblur={sb_args}')
        record('smartblur matches FFmpeg pixel-exact',stats=stats)
        plan=native('plan',source,'--smartblur',sb_args)
        assert plan['graph']==f'smartblur={sb_args}'
        record('plan explains smartblur graph',plan=plan)
        sab_args='lr=2:cr=2'
        out=w/'sab.mkv';stats=native('transcode-lossless',source,out,'--sab',sab_args)
        assert pixels(out)==pixels(source,f'sab={sab_args}')
        record('sab matches FFmpeg pixel-exact',stats=stats)
        plan=native('plan',source,'--sab',sab_args)
        assert plan['graph']==f'sab={sab_args}'
        record('plan explains sab graph',plan=plan)
        bi_args='sigmaS=0.1:sigmaR=0.1'
        out=w/'bilateral.mkv';stats=native('transcode-lossless',source,out,'--bilateral',bi_args)
        assert pixels(out)==pixels(source,f'bilateral={bi_args}')
        record('bilateral matches FFmpeg pixel-exact',stats=stats)
        plan=native('plan',source,'--bilateral',bi_args)
        assert plan['graph']==f'bilateral={bi_args}'
        record('plan explains bilateral graph',plan=plan)
        cas_args='strength=0.5'
        out=w/'cas.mkv';stats=native('transcode-lossless',source,out,'--cas',cas_args)
        assert pixels(out)==pixels(source,f'cas={cas_args}')
        record('cas matches FFmpeg pixel-exact',stats=stats)
        plan=native('plan',source,'--cas',cas_args)
        assert plan['graph']==f'cas={cas_args}'
        record('plan explains cas graph',plan=plan)
        epx_src=w/'epx-src.mp4'
        ff('-f','lavfi','-i','testsrc=size=64x36:rate=25','-t','0.4','-an','-c:v','libx264','-bf','0','-g','25','-keyint_min','25','-sc_threshold','0','-pix_fmt','yuv420p',epx_src)
        epx_args='n=2'
        out=w/'epx.mkv';stats=native('transcode-lossless',epx_src,out,'--epx',epx_args)
        assert pixels(out)==pixels(epx_src,f'epx={epx_args}')
        record('epx matches FFmpeg pixel-exact',stats=stats)
        plan=native('plan',epx_src,'--epx',epx_args)
        assert plan['graph']==f'epx={epx_args}'
        record('plan explains epx graph',plan=plan)
        stats=native('decode',epx_src,'--epx',epx_args)
        assert stats['width']==128 and stats['height']==72 and stats['video_frames']==10
        record('decode reports epx geometry',stats=stats)
        vig_args='angle=PI/4'
        out=w/'vignette.mkv';stats=native('transcode-lossless',source,out,'--vignette',vig_args)
        assert pixels(out)==pixels(source,f'vignette={vig_args}')
        record('vignette matches FFmpeg pixel-exact',stats=stats)
        plan=native('plan',source,'--vignette',vig_args)
        assert plan['graph']==f'vignette={vig_args}'
        record('plan explains vignette graph',plan=plan)
        curves_args='preset=vintage'
        out=w/'curves.mkv';stats=native('transcode-lossless',source,out,'--curves',curves_args)
        assert pixels(out)==pixels(source,f'curves={curves_args}')
        record('curves matches FFmpeg pixel-exact',stats=stats)
        plan=native('plan',source,'--curves',curves_args)
        assert plan['graph']==f'curves={curves_args}'
        record('plan explains curves graph',plan=plan)
        cb_args='rs=.1:gs=.05:bs=-.1'
        out=w/'colorbalance.mkv';stats=native('transcode-lossless',source,out,'--colorbalance',cb_args)
        assert pixels(out)==pixels(source,f'colorbalance={cb_args}')
        record('colorbalance matches FFmpeg pixel-exact',stats=stats)
        plan=native('plan',source,'--colorbalance',cb_args)
        assert plan['graph']==f'colorbalance={cb_args}'
        record('plan explains colorbalance graph',plan=plan)
        cl_args='rimin=0.1:gimin=0.1:bimin=0.1'
        out=w/'colorlevels.mkv';stats=native('transcode-lossless',source,out,'--colorlevels',cl_args)
        assert pixels(out)==pixels(source,f'colorlevels={cl_args}')
        record('colorlevels matches FFmpeg pixel-exact',stats=stats)
        plan=native('plan',source,'--colorlevels',cl_args)
        assert plan['graph']==f'colorlevels={cl_args}'
        record('plan explains colorlevels graph',plan=plan)
        ccm_args='rr=1.1:gg=0.9:bb=1.0'
        out=w/'colorchannelmixer.mkv';stats=native('transcode-lossless',source,out,'--colorchannelmixer',ccm_args)
        assert pixels(out)==pixels(source,f'colorchannelmixer={ccm_args}')
        record('colorchannelmixer matches FFmpeg pixel-exact',stats=stats)
        plan=native('plan',source,'--colorchannelmixer',ccm_args)
        assert plan['graph']==f'colorchannelmixer={ccm_args}'
        record('plan explains colorchannelmixer graph',plan=plan)
        df_args='mode=am:size=5'
        out=w/'deflicker.mkv';stats=native('transcode-lossless',source,out,'--deflicker',df_args)
        assert pixels(out)==pixels(source,f'deflicker={df_args}')
        assert stats['video_frames']==50  # 2s@25fps → deflicker flush restores 1:1
        record('deflicker matches FFmpeg pixel-exact',stats=stats)
        plan=native('plan',source,'--deflicker',df_args)
        assert plan['graph']==f'deflicker={df_args}'
        record('plan explains deflicker graph',plan=plan)
        ps_args='f=5'
        out=w/'photosensitivity.mkv';stats=native('transcode-lossless',source,out,'--photosensitivity',ps_args)
        assert pixels(out)==pixels(source,f'photosensitivity={ps_args}')
        assert stats['video_frames']==50
        record('photosensitivity matches FFmpeg pixel-exact',stats=stats)
        plan=native('plan',source,'--photosensitivity',ps_args)
        assert plan['graph']==f'photosensitivity={ps_args}'
        record('plan explains photosensitivity graph',plan=plan)
        mc_args='cb=0.2:cr=-0.1:size=1.5:high=0.3'
        out=w/'monochrome.mkv';stats=native('transcode-lossless',source,out,'--monochrome',mc_args)
        assert pixels(out)==pixels(source,f'monochrome={mc_args}')
        record('monochrome matches FFmpeg pixel-exact',stats=stats)
        plan=native('plan',source,'--monochrome',mc_args)
        assert plan['graph']==f'monochrome={mc_args}'
        record('plan explains monochrome graph',plan=plan)
        out=w/'grayworld.mkv';stats=native('transcode-lossless',source,out,'--grayworld','0')
        assert pixels(out)==pixels(source,'grayworld')
        record('grayworld matches FFmpeg pixel-exact',stats=stats)
        plan=native('plan',source,'--grayworld','0')
        assert plan['graph']=='grayworld'
        record('plan explains grayworld graph',plan=plan)
        drawbox_src=w/'drawbox-src.mp4'
        ff('-f','lavfi','-i','testsrc=size=128x72:rate=25','-t','0.4','-an','-c:v','libx264','-bf','0','-g','25','-keyint_min','25','-sc_threshold','0','-pix_fmt','yuv420p',drawbox_src)
        db_args='x=10:y=10:w=40:h=20:color=red'
        out=w/'drawbox.mkv';stats=native('transcode-lossless',drawbox_src,out,'--drawbox',db_args)
        assert pixels(out)==pixels(drawbox_src,f'drawbox={db_args}')
        assert stats['video_frames']==10
        record('drawbox matches FFmpeg pixel-exact',stats=stats)
        plan=native('plan',drawbox_src,'--drawbox',db_args)
        assert plan['graph']==f'drawbox={db_args}'
        record('plan explains drawbox graph',plan=plan)
        drawgrid_src=w/'drawgrid-src.mp4'
        ff('-f','lavfi','-i','testsrc=size=128x72:rate=25','-t','0.4','-an','-c:v','libx264','-bf','0','-g','25','-keyint_min','25','-sc_threshold','0','-pix_fmt','yuv420p',drawgrid_src)
        dg_args='w=16:h=16:color=white'
        out=w/'drawgrid.mkv';stats=native('transcode-lossless',drawgrid_src,out,'--drawgrid',dg_args)
        assert pixels(out)==pixels(drawgrid_src,f'drawgrid={dg_args}')
        assert stats['video_frames']==10
        record('drawgrid matches FFmpeg pixel-exact',stats=stats)
        plan=native('plan',drawgrid_src,'--drawgrid',dg_args)
        assert plan['graph']==f'drawgrid={dg_args}'
        record('plan explains drawgrid graph',plan=plan)
        lagfun_src=w/'lagfun-src.mp4'
        ff('-f','lavfi','-i','testsrc=size=128x72:rate=25','-t','0.4','-an','-c:v','libx264','-bf','0','-g','25','-keyint_min','25','-sc_threshold','0','-pix_fmt','yuv420p',lagfun_src)
        lf_args='decay=0.95'
        out=w/'lagfun.mkv';stats=native('transcode-lossless',lagfun_src,out,'--lagfun',lf_args)
        assert pixels(out)==pixels(lagfun_src,f'lagfun={lf_args}')
        assert stats['video_frames']==10
        record('lagfun matches FFmpeg pixel-exact',stats=stats)
        plan=native('plan',lagfun_src,'--lagfun',lf_args)
        assert plan['graph']==f'lagfun={lf_args}'
        record('plan explains lagfun graph',plan=plan)
        bitplanenoise_src=w/'bitplanenoise-src.mp4'
        ff('-f','lavfi','-i','testsrc=size=128x72:rate=25','-t','0.4','-an','-c:v','libx264','-bf','0','-g','25','-keyint_min','25','-sc_threshold','0','-pix_fmt','yuv420p',bitplanenoise_src)
        bn_args='bitplane=1:filter=1'
        out=w/'bitplanenoise.mkv';stats=native('transcode-lossless',bitplanenoise_src,out,'--bitplanenoise',bn_args)
        assert pixels(out)==pixels(bitplanenoise_src,f'bitplanenoise={bn_args}')
        assert stats['video_frames']==10
        record('bitplanenoise matches FFmpeg pixel-exact',stats=stats)
        plan=native('plan',bitplanenoise_src,'--bitplanenoise',bn_args)
        assert plan['graph']==f'bitplanenoise={bn_args}'
        record('plan explains bitplanenoise graph',plan=plan)
        deband_src=w/'deband-src.mp4'
        ff('-f','lavfi','-i','testsrc=size=128x72:rate=25','-t','0.4','-an','-c:v','libx264','-bf','0','-g','25','-keyint_min','25','-sc_threshold','0','-pix_fmt','yuv420p',deband_src)
        db_args='1thr=0.02'
        out=w/'deband.mkv';stats=native('transcode-lossless',deband_src,out,'--deband',db_args)
        assert pixels(out)==pixels(deband_src,f'deband={db_args}')
        assert stats['video_frames']==10
        record('deband matches FFmpeg pixel-exact',stats=stats)
        plan=native('plan',deband_src,'--deband',db_args)
        assert plan['graph']==f'deband={db_args}'
        record('plan explains deband graph',plan=plan)
        gradfun_src=w/'gradfun-src.mp4'
        ff('-f','lavfi','-i','testsrc=size=128x72:rate=25','-t','0.4','-an','-c:v','libx264','-bf','0','-g','25','-keyint_min','25','-sc_threshold','0','-pix_fmt','yuv420p',gradfun_src)
        gf_args='strength=1.2'
        out=w/'gradfun.mkv';stats=native('transcode-lossless',gradfun_src,out,'--gradfun',gf_args)
        assert pixels(out)==pixels(gradfun_src,f'gradfun={gf_args}')
        assert stats['video_frames']==10
        record('gradfun matches FFmpeg pixel-exact',stats=stats)
        plan=native('plan',gradfun_src,'--gradfun',gf_args)
        assert plan['graph']==f'gradfun={gf_args}'
        record('plan explains gradfun graph',plan=plan)
        lenscorrection_src=w/'lenscorrection-src.mp4'
        ff('-f','lavfi','-i','testsrc=size=128x72:rate=25','-t','0.4','-an','-c:v','libx264','-bf','0','-g','25','-keyint_min','25','-sc_threshold','0','-pix_fmt','yuv420p',lenscorrection_src)
        lc_args='k1=-0.1'
        out=w/'lenscorrection.mkv';stats=native('transcode-lossless',lenscorrection_src,out,'--lenscorrection',lc_args)
        assert pixels(out)==pixels(lenscorrection_src,f'lenscorrection={lc_args}')
        assert stats['video_frames']==10
        record('lenscorrection matches FFmpeg pixel-exact',stats=stats)
        plan=native('plan',lenscorrection_src,'--lenscorrection',lc_args)
        assert plan['graph']==f'lenscorrection={lc_args}'
        record('plan explains lenscorrection graph',plan=plan)
        pixelize_src=w/'pixelize-src.mp4'
        ff('-f','lavfi','-i','testsrc=size=128x72:rate=25','-t','0.4','-an','-c:v','libx264','-bf','0','-g','25','-keyint_min','25','-sc_threshold','0','-pix_fmt','yuv420p',pixelize_src)
        pz_args='width=8:height=8'
        out=w/'pixelize.mkv';stats=native('transcode-lossless',pixelize_src,out,'--pixelize',pz_args)
        assert pixels(out)==pixels(pixelize_src,f'pixelize={pz_args}')
        assert stats['video_frames']==10
        record('pixelize matches FFmpeg pixel-exact',stats=stats)
        plan=native('plan',pixelize_src,'--pixelize',pz_args)
        assert plan['graph']==f'pixelize={pz_args}'
        record('plan explains pixelize graph',plan=plan)
        removegrain_src=w/'removegrain-src.mp4'
        ff('-f','lavfi','-i','testsrc=size=128x72:rate=25','-t','0.4','-an','-c:v','libx264','-bf','0','-g','25','-keyint_min','25','-sc_threshold','0','-pix_fmt','yuv420p',removegrain_src)
        rg_args='m0=1'
        out=w/'removegrain.mkv';stats=native('transcode-lossless',removegrain_src,out,'--removegrain',rg_args)
        assert pixels(out)==pixels(removegrain_src,f'removegrain={rg_args}')
        assert stats['video_frames']==10
        record('removegrain matches FFmpeg pixel-exact',stats=stats)
        plan=native('plan',removegrain_src,'--removegrain',rg_args)
        assert plan['graph']==f'removegrain={rg_args}'
        record('plan explains removegrain graph',plan=plan)
        yaepblur_src=w/'yaepblur-src.mp4'
        ff('-f','lavfi','-i','testsrc=size=128x72:rate=25','-t','0.4','-an','-c:v','libx264','-bf','0','-g','25','-keyint_min','25','-sc_threshold','0','-pix_fmt','yuv420p',yaepblur_src)
        yb_args='r=3'
        out=w/'yaepblur.mkv';stats=native('transcode-lossless',yaepblur_src,out,'--yaepblur',yb_args)
        assert pixels(out)==pixels(yaepblur_src,f'yaepblur={yb_args}')
        assert stats['video_frames']==10
        record('yaepblur matches FFmpeg pixel-exact',stats=stats)
        plan=native('plan',yaepblur_src,'--yaepblur',yb_args)
        assert plan['graph']==f'yaepblur={yb_args}'
        record('plan explains yaepblur graph',plan=plan)
        vibrance_src=w/'vibrance-src.mp4'
        ff('-f','lavfi','-i','testsrc=size=128x72:rate=25','-t','0.4','-an','-c:v','libx264','-bf','0','-g','25','-keyint_min','25','-sc_threshold','0','-pix_fmt','yuv420p',vibrance_src)
        vib_args='intensity=0.3:rbal=1'
        out=w/'vibrance.mkv';stats=native('transcode-lossless',vibrance_src,out,'--vibrance',vib_args)
        assert pixels(out)==pixels(vibrance_src,f'vibrance={vib_args}')
        assert stats['video_frames']==10
        record('vibrance matches FFmpeg pixel-exact',stats=stats)
        plan=native('plan',vibrance_src,'--vibrance',vib_args)
        assert plan['graph']==f'vibrance={vib_args}'
        record('plan explains vibrance graph',plan=plan)
        dilation_src=w/'dilation-src.mp4'
        ff('-f','lavfi','-i','testsrc=size=128x72:rate=25','-t','0.4','-an','-c:v','libx264','-bf','0','-g','25','-keyint_min','25','-sc_threshold','0','-pix_fmt','yuv420p',dilation_src)
        dil_args='threshold0=10'
        out=w/'dilation.mkv';stats=native('transcode-lossless',dilation_src,out,'--dilation',dil_args)
        assert pixels(out)==pixels(dilation_src,f'dilation={dil_args}')
        assert stats['video_frames']==10
        record('dilation matches FFmpeg pixel-exact',stats=stats)
        plan=native('plan',dilation_src,'--dilation',dil_args)
        assert plan['graph']==f'dilation={dil_args}'
        record('plan explains dilation graph',plan=plan)
        erosion_src=w/'erosion-src.mp4'
        ff('-f','lavfi','-i','testsrc=size=128x72:rate=25','-t','0.4','-an','-c:v','libx264','-bf','0','-g','25','-keyint_min','25','-sc_threshold','0','-pix_fmt','yuv420p',erosion_src)
        ero_args='threshold0=10'
        out=w/'erosion.mkv';stats=native('transcode-lossless',erosion_src,out,'--erosion',ero_args)
        assert pixels(out)==pixels(erosion_src,f'erosion={ero_args}')
        assert stats['video_frames']==10
        record('erosion matches FFmpeg pixel-exact',stats=stats)
        plan=native('plan',erosion_src,'--erosion',ero_args)
        assert plan['graph']==f'erosion={ero_args}'
        record('plan explains erosion graph',plan=plan)
        colorize_src=w/'colorize-src.mp4'
        ff('-f','lavfi','-i','testsrc=size=128x72:rate=25','-t','0.4','-an','-c:v','libx264','-bf','0','-g','25','-keyint_min','25','-sc_threshold','0','-pix_fmt','yuv420p',colorize_src)
        col_args='hue=120:saturation=0.5'
        out=w/'colorize.mkv';stats=native('transcode-lossless',colorize_src,out,'--colorize',col_args)
        assert pixels(out)==pixels(colorize_src,f'colorize={col_args}')
        assert stats['video_frames']==10
        record('colorize matches FFmpeg pixel-exact',stats=stats)
        plan=native('plan',colorize_src,'--colorize',col_args)
        assert plan['graph']==f'colorize={col_args}'
        record('plan explains colorize graph',plan=plan)
        exposure_src=w/'exposure-src.mp4'
        ff('-f','lavfi','-i','testsrc=size=128x72:rate=25','-t','0.4','-an','-c:v','libx264','-bf','0','-g','25','-keyint_min','25','-sc_threshold','0','-pix_fmt','yuv420p',exposure_src)
        exp_args='exposure=0.5'
        out=w/'exposure.mkv';stats=native('transcode-lossless',exposure_src,out,'--exposure',exp_args)
        assert pixels(out)==pixels(exposure_src,f'exposure={exp_args}')
        assert stats['video_frames']==10
        record('exposure matches FFmpeg pixel-exact',stats=stats)
        plan=native('plan',exposure_src,'--exposure',exp_args)
        assert plan['graph']==f'exposure={exp_args}'
        record('plan explains exposure graph',plan=plan)
        chromashift_src=w/'chromashift-src.mp4'
        ff('-f','lavfi','-i','testsrc=size=128x72:rate=25','-t','0.4','-an','-c:v','libx264','-bf','0','-g','25','-keyint_min','25','-sc_threshold','0','-pix_fmt','yuv420p',chromashift_src)
        cs_args='cbh=4'
        out=w/'chromashift.mkv';stats=native('transcode-lossless',chromashift_src,out,'--chromashift',cs_args)
        assert pixels(out)==pixels(chromashift_src,f'chromashift={cs_args}')
        assert stats['video_frames']==10
        record('chromashift matches FFmpeg pixel-exact',stats=stats)
        plan=native('plan',chromashift_src,'--chromashift',cs_args)
        assert plan['graph']==f'chromashift={cs_args}'
        record('plan explains chromashift graph',plan=plan)
        colorcontrast_src=w/'colorcontrast-src.mp4'
        ff('-f','lavfi','-i','testsrc=size=128x72:rate=25','-t','0.4','-an','-c:v','libx264','-bf','0','-g','25','-keyint_min','25','-sc_threshold','0','-pix_fmt','yuv420p',colorcontrast_src)
        cc_args='rc=0.1:gm=0.1:by=0.1'
        out=w/'colorcontrast.mkv';stats=native('transcode-lossless',colorcontrast_src,out,'--colorcontrast',cc_args)
        assert pixels(out)==pixels(colorcontrast_src,f'colorcontrast={cc_args}')
        assert stats['video_frames']==10
        record('colorcontrast matches FFmpeg pixel-exact',stats=stats)
        plan=native('plan',colorcontrast_src,'--colorcontrast',cc_args)
        assert plan['graph']==f'colorcontrast={cc_args}'
        record('plan explains colorcontrast graph',plan=plan)
        colorcorrect_src=w/'colorcorrect-src.mp4'
        ff('-f','lavfi','-i','testsrc=size=128x72:rate=25','-t','0.4','-an','-c:v','libx264','-bf','0','-g','25','-keyint_min','25','-sc_threshold','0','-pix_fmt','yuv420p',colorcorrect_src)
        cr_args='rl=0.1:bl=-0.1'
        out=w/'colorcorrect.mkv';stats=native('transcode-lossless',colorcorrect_src,out,'--colorcorrect',cr_args)
        assert pixels(out)==pixels(colorcorrect_src,f'colorcorrect={cr_args}')
        assert stats['video_frames']==10
        record('colorcorrect matches FFmpeg pixel-exact',stats=stats)
        plan=native('plan',colorcorrect_src,'--colorcorrect',cr_args)
        assert plan['graph']==f'colorcorrect={cr_args}'
        record('plan explains colorcorrect graph',plan=plan)
        histeq_src=w/'histeq-src.mp4'
        ff('-f','lavfi','-i','testsrc=size=128x72:rate=25','-t','0.4','-an','-c:v','libx264','-bf','0','-g','25','-keyint_min','25','-sc_threshold','0','-pix_fmt','yuv420p',histeq_src)
        he_args='strength=0.2'
        out=w/'histeq.mkv';stats=native('transcode-lossless',histeq_src,out,'--histeq',he_args)
        assert pixels(out)==pixels(histeq_src,f'histeq={he_args}')
        assert stats['video_frames']==10
        record('histeq matches FFmpeg pixel-exact',stats=stats)
        plan=native('plan',histeq_src,'--histeq',he_args)
        assert plan['graph']==f'histeq={he_args}'
        record('plan explains histeq graph',plan=plan)
        shuffle_src=w/'shuffle-src.mp4'
        ff('-f','lavfi','-i','testsrc=size=128x72:rate=25','-t','0.4','-an','-c:v','libx264','-bf','0','-g','25','-keyint_min','25','-sc_threshold','0','-pix_fmt','yuv420p',shuffle_src)
        sp_args='map0=0:map1=2:map2=1'
        out=w/'shuffleplanes.mkv';stats=native('transcode-lossless',shuffle_src,out,'--shuffleplanes',sp_args)
        assert pixels(out)==pixels(shuffle_src,f'shuffleplanes={sp_args}')
        assert stats['video_frames']==10
        record('shuffleplanes matches FFmpeg pixel-exact',stats=stats)
        plan=native('plan',shuffle_src,'--shuffleplanes',sp_args)
        assert plan['graph']==f'shuffleplanes={sp_args}'
        record('plan explains shuffleplanes graph',plan=plan)
        lutyuv_src=w/'lutyuv-src.mp4'
        ff('-f','lavfi','-i','testsrc=size=128x72:rate=25','-t','0.4','-an','-c:v','libx264','-bf','0','-g','25','-keyint_min','25','-sc_threshold','0','-pix_fmt','yuv420p',lutyuv_src)
        ly_args='y=val*0.8'
        out=w/'lutyuv.mkv';stats=native('transcode-lossless',lutyuv_src,out,'--lutyuv',ly_args)
        assert pixels(out)==pixels(lutyuv_src,f'lutyuv={ly_args}')
        assert stats['video_frames']==10
        record('lutyuv matches FFmpeg pixel-exact',stats=stats)
        plan=native('plan',lutyuv_src,'--lutyuv',ly_args)
        assert plan['graph']==f'lutyuv={ly_args}'
        record('plan explains lutyuv graph',plan=plan)
        colorhold_src=w/'colorhold-src.mp4'
        ff('-f','lavfi','-i','testsrc=size=128x72:rate=25','-t','0.4','-an','-c:v','libx264','-bf','0','-g','25','-keyint_min','25','-sc_threshold','0','-pix_fmt','yuv420p',colorhold_src)
        ch_args='similarity=0.2:blend=0.1'
        out=w/'colorhold.mkv';stats=native('transcode-lossless',colorhold_src,out,'--colorhold',ch_args)
        assert pixels(out)==pixels(colorhold_src,f'colorhold={ch_args}')
        assert stats['video_frames']==10
        record('colorhold matches FFmpeg pixel-exact',stats=stats)
        plan=native('plan',colorhold_src,'--colorhold',ch_args)
        assert plan['graph']==f'colorhold={ch_args}'
        record('plan explains colorhold graph',plan=plan)
        fade_src=w/'fade-src.mp4'
        ff('-f','lavfi','-i','testsrc=size=128x72:rate=25','-t','0.4','-an','-c:v','libx264','-bf','0','-g','25','-keyint_min','25','-sc_threshold','0','-pix_fmt','yuv420p',fade_src)
        fd_args='t=in:s=0:n=4'
        out=w/'fade.mkv';stats=native('transcode-lossless',fade_src,out,'--fade',fd_args)
        assert pixels(out)==pixels(fade_src,f'fade={fd_args}')
        assert stats['video_frames']==10
        record('fade matches FFmpeg pixel-exact',stats=stats)
        plan=native('plan',fade_src,'--fade',fd_args)
        assert plan['graph']==f'fade={fd_args}'
        record('plan explains fade graph',plan=plan)
        perspective_src=w/'perspective-src.mp4'
        ff('-f','lavfi','-i','testsrc=size=128x72:rate=25','-t','0.4','-an','-c:v','libx264','-bf','0','-g','25','-keyint_min','25','-sc_threshold','0','-pix_fmt','yuv420p',perspective_src)
        ps_args='sense=destination:x0=0:y0=10:x1=W:y1=0:x2=0:y2=H:x3=W:y3=H-10'
        out=w/'perspective.mkv';stats=native('transcode-lossless',perspective_src,out,'--perspective',ps_args)
        assert pixels(out)==pixels(perspective_src,f'perspective={ps_args}')
        assert stats['video_frames']==10
        record('perspective matches FFmpeg pixel-exact',stats=stats)
        plan=native('plan',perspective_src,'--perspective',ps_args)
        assert plan['graph']==f'perspective={ps_args}'
        record('plan explains perspective graph',plan=plan)
        lumakey_src=w/'lumakey-src.mp4'
        ff('-f','lavfi','-i','testsrc=size=128x72:rate=25','-t','0.4','-an','-c:v','libx264','-bf','0','-g','25','-keyint_min','25','-sc_threshold','0','-pix_fmt','yuv420p',lumakey_src)
        lk_args='threshold=0.1:tolerance=0.1:softness=0.1'
        out=w/'lumakey.mkv';stats=native('transcode-lossless',lumakey_src,out,'--lumakey',lk_args)
        assert pixels(out,pixfmt='yuva420p')==pixels(lumakey_src,f'lumakey={lk_args}',pixfmt='yuva420p')
        assert stats['video_frames']==10
        record('lumakey matches FFmpeg pixel-exact',stats=stats)
        plan=native('plan',lumakey_src,'--lumakey',lk_args)
        assert plan['graph']==f'lumakey={lk_args}'
        record('plan explains lumakey graph',plan=plan)
        chromakey_src=w/'chromakey-src.mp4'
        ff('-f','lavfi','-i','testsrc=size=128x72:rate=25','-t','0.4','-an','-c:v','libx264','-bf','0','-g','25','-keyint_min','25','-sc_threshold','0','-pix_fmt','yuv420p',chromakey_src)
        ck_args='similarity=0.3:blend=0.1'
        out=w/'chromakey.mkv';stats=native('transcode-lossless',chromakey_src,out,'--chromakey',ck_args)
        assert pixels(out,pixfmt='yuva420p')==pixels(chromakey_src,f'chromakey={ck_args}',pixfmt='yuva420p')
        assert stats['video_frames']==10
        record('chromakey matches FFmpeg pixel-exact',stats=stats)
        plan=native('plan',chromakey_src,'--chromakey',ck_args)
        assert plan['graph']==f'chromakey={ck_args}'
        record('plan explains chromakey graph',plan=plan)
        colorkey_src=w/'colorkey-src.mp4'
        ff('-f','lavfi','-i','testsrc=size=128x72:rate=25','-t','0.4','-an','-c:v','libx264','-bf','0','-g','25','-keyint_min','25','-sc_threshold','0','-pix_fmt','yuv420p',colorkey_src)
        ck_args='color=black:similarity=0.1:blend=0.1'
        out=w/'colorkey.mkv';stats=native('transcode-lossless',colorkey_src,out,'--colorkey',ck_args)
        assert pixels(out,pixfmt='yuva420p')==pixels(colorkey_src,f'colorkey={ck_args},format=yuva420p',pixfmt='yuva420p')
        assert stats['video_frames']==10
        record('colorkey matches FFmpeg pixel-exact',stats=stats)
        plan=native('plan',colorkey_src,'--colorkey',ck_args)
        assert plan['graph']==f'colorkey={ck_args}'
        record('plan explains colorkey graph',plan=plan)
        despill_src=w/'despill-src.mp4'
        ff('-f','lavfi','-i','testsrc=size=128x72:rate=25','-t','0.4','-an','-c:v','libx264','-bf','0','-g','25','-keyint_min','25','-sc_threshold','0','-pix_fmt','yuv420p',despill_src)
        ds_args='type=green:mix=0.5'
        out=w/'despill.mkv';stats=native('transcode-lossless',despill_src,out,'--despill',ds_args)
        assert pixels(out)==pixels(despill_src,f'despill={ds_args}')
        assert stats['video_frames']==10
        record('despill matches FFmpeg pixel-exact',stats=stats)
        plan=native('plan',despill_src,'--despill',ds_args)
        assert plan['graph']==f'despill={ds_args}'
        record('plan explains despill graph',plan=plan)
        selectivecolor_src=w/'selectivecolor-src.mp4'
        ff('-f','lavfi','-i','testsrc=size=128x72:rate=25','-t','0.4','-an','-c:v','libx264','-bf','0','-g','25','-keyint_min','25','-sc_threshold','0','-pix_fmt','yuv420p',selectivecolor_src)
        sc_args='reds=0.2 0 0 0'
        out=w/'selectivecolor.mkv';stats=native('transcode-lossless',selectivecolor_src,out,'--selectivecolor',sc_args)
        assert pixels(out)==pixels(selectivecolor_src,f'selectivecolor={sc_args}')
        assert stats['video_frames']==10
        record('selectivecolor matches FFmpeg pixel-exact',stats=stats)
        plan=native('plan',selectivecolor_src,'--selectivecolor',sc_args)
        assert plan['graph']==f'selectivecolor={sc_args}'
        record('plan explains selectivecolor graph',plan=plan)
        stereo3d_src=w/'stereo3d-src.mp4'
        ff('-f','lavfi','-i','testsrc=size=128x72:rate=25','-t','0.4','-an','-c:v','libx264','-bf','0','-g','25','-keyint_min','25','-sc_threshold','0','-pix_fmt','yuv420p',stereo3d_src)
        s3d_args='sbsl:abl'
        out=w/'stereo3d.mkv';stats=native('transcode-lossless',stereo3d_src,out,'--stereo3d',s3d_args)
        assert pixels(out)==pixels(stereo3d_src,f'stereo3d={s3d_args}')
        assert stats['video_frames']==10
        record('stereo3d matches FFmpeg pixel-exact',stats=stats)
        plan=native('plan',stereo3d_src,'--stereo3d',s3d_args)
        assert plan['graph']==f'stereo3d={s3d_args}'
        record('plan explains stereo3d graph',plan=plan)
        field_src=w/'field-src.mp4'
        ff('-f','lavfi','-i','testsrc=size=128x72:rate=25','-t','0.4','-an','-c:v','libx264','-bf','0','-g','25','-keyint_min','25','-sc_threshold','0','-pix_fmt','yuv420p',field_src)
        field_args='bottom'
        out=w/'field.mkv';stats=native('transcode-lossless',field_src,out,'--field',field_args)
        assert pixels(out)==pixels(field_src,f'field={field_args}')
        assert stats['video_frames']==10
        record('field matches FFmpeg pixel-exact',stats=stats)
        plan=native('plan',field_src,'--field',field_args)
        assert plan['graph']==f'field={field_args}'
        record('plan explains field graph',plan=plan)
        hqx_src=w/'hqx-src.mp4'
        ff('-f','lavfi','-i','testsrc=size=64x36:rate=25','-t','0.4','-an','-c:v','libx264','-bf','0','-g','25','-keyint_min','25','-sc_threshold','0','-pix_fmt','yuv420p',hqx_src)
        hqx_args='n=2'
        out=w/'hqx.mkv';stats=native('transcode-lossless',hqx_src,out,'--hqx',hqx_args)
        assert pixels(out)==pixels(hqx_src,f'format=argb,hqx={hqx_args},format=yuv420p')
        assert stats['video_frames']==10
        record('hqx matches FFmpeg pixel-exact',stats=stats)
        plan=native('plan',hqx_src,'--hqx',hqx_args)
        assert plan['graph']==f'hqx={hqx_args}'
        record('plan explains hqx graph',plan=plan)
        xbr_src=w/'xbr-src.mp4'
        ff('-f','lavfi','-i','testsrc=size=64x36:rate=25','-t','0.4','-an','-c:v','libx264','-bf','0','-g','25','-keyint_min','25','-sc_threshold','0','-pix_fmt','yuv420p',xbr_src)
        xbr_args='n=2'
        out=w/'xbr.mkv';stats=native('transcode-lossless',xbr_src,out,'--xbr',xbr_args)
        assert pixels(out)==pixels(xbr_src,f'format=argb,xbr={xbr_args},format=yuv420p')
        assert stats['video_frames']==10
        record('xbr matches FFmpeg pixel-exact',stats=stats)
        plan=native('plan',xbr_src,'--xbr',xbr_args)
        assert plan['graph']==f'xbr={xbr_args}'
        record('plan explains xbr graph',plan=plan)
        il_src=w/'il-src.mp4'
        ff('-f','lavfi','-i','testsrc=size=64x36:rate=25','-t','0.4','-an','-c:v','libx264','-bf','0','-g','25','-keyint_min','25','-sc_threshold','0','-pix_fmt','yuv420p',il_src)
        il_args='l=d:c=d'
        out=w/'il.mkv';stats=native('transcode-lossless',il_src,out,'--il',il_args)
        assert pixels(out)==pixels(il_src,f'il={il_args}')
        assert stats['video_frames']==10
        record('il matches FFmpeg pixel-exact',stats=stats)
        plan=native('plan',il_src,'--il',il_args)
        assert plan['graph']==f'il={il_args}'
        record('plan explains il graph',plan=plan)
        super2xsai_src=w/'super2xsai-src.mp4'
        ff('-f','lavfi','-i','testsrc=size=64x36:rate=25','-t','0.4','-an','-c:v','libx264','-bf','0','-g','25','-keyint_min','25','-sc_threshold','0','-pix_fmt','yuv420p',super2xsai_src)
        super2xsai_args=''
        out=w/'super2xsai.mkv';stats=native('transcode-lossless',super2xsai_src,out,'--super2xsai',super2xsai_args)
        assert pixels(out)==pixels(super2xsai_src,'format=argb,super2xsai,format=yuv420p')
        assert stats['video_frames']==10
        record('super2xsai matches FFmpeg pixel-exact',stats=stats)
        plan=native('plan',super2xsai_src,'--super2xsai',super2xsai_args)
        assert plan['graph']=='super2xsai'
        record('plan explains super2xsai graph',plan=plan)
        kerndeint_src=w/'kerndeint-src.mp4'
        ff('-f','lavfi','-i','testsrc=size=64x36:rate=25','-t','0.4','-an','-c:v','libx264','-bf','0','-g','25','-keyint_min','25','-sc_threshold','0','-pix_fmt','yuv420p',kerndeint_src)
        kerndeint_args='thresh=10'
        out=w/'kerndeint.mkv';stats=native('transcode-lossless',kerndeint_src,out,'--kerndeint',kerndeint_args)
        assert pixels(out)==pixels(kerndeint_src,f'kerndeint={kerndeint_args}')
        assert stats['video_frames']==10
        record('kerndeint matches FFmpeg pixel-exact',stats=stats)
        plan=native('plan',kerndeint_src,'--kerndeint',kerndeint_args)
        assert plan['graph']==f'kerndeint={kerndeint_args}'
        record('plan explains kerndeint graph',plan=plan)
        phase_src=w/'phase-src.mp4'
        ff('-f','lavfi','-i','testsrc=size=128x72:rate=25','-t','0.4','-an','-c:v','libx264','-bf','0','-g','25','-keyint_min','25','-sc_threshold','0','-pix_fmt','yuv420p',phase_src)
        phase_args='mode=t'
        out=w/'phase.mkv';stats=native('transcode-lossless',phase_src,out,'--phase',phase_args)
        assert pixels(out)==pixels(phase_src,f'phase={phase_args}')
        assert stats['video_frames']==10
        record('phase matches FFmpeg pixel-exact',stats=stats)
        plan=native('plan',phase_src,'--phase',phase_args)
        assert plan['graph']==f'phase={phase_args}'
        record('plan explains phase graph',plan=plan)
        estdif_src=w/'estdif-src.mp4'
        ff('-f','lavfi','-i','testsrc=size=64x36:rate=25','-t','0.4','-an','-c:v','libx264','-bf','0','-g','25','-keyint_min','25','-sc_threshold','0','-pix_fmt','yuv420p',estdif_src)
        estdif_args='mode=frame'
        out=w/'estdif.mkv';stats=native('transcode-lossless',estdif_src,out,'--estdif',estdif_args)
        assert pixels(out)==pixels(estdif_src,f'estdif={estdif_args}')
        assert stats['video_frames']==10
        record('estdif matches FFmpeg pixel-exact',stats=stats)
        plan=native('plan',estdif_src,'--estdif',estdif_args)
        assert plan['graph']==f'estdif={estdif_args}'
        record('plan explains estdif graph',plan=plan)
        tinterlace_src=w/'tinterlace-src.mp4'
        ff('-f','lavfi','-i','testsrc=size=64x36:rate=25','-t','0.4','-an','-c:v','libx264','-bf','0','-g','25','-keyint_min','25','-sc_threshold','0','-pix_fmt','yuv420p',tinterlace_src)
        tinterlace_args='mode=interleave_top'
        out=w/'tinterlace.mkv';stats=native('transcode-lossless',tinterlace_src,out,'--tinterlace',tinterlace_args)
        assert pixels(out)==pixels(tinterlace_src,f'tinterlace={tinterlace_args}')
        assert stats['video_frames']==5
        record('tinterlace matches FFmpeg pixel-exact',stats=stats)
        plan=native('plan',tinterlace_src,'--tinterlace',tinterlace_args)
        assert plan['graph']==f'tinterlace={tinterlace_args}'
        record('plan explains tinterlace graph',plan=plan)
        separatefields_src=w/'separatefields-src.mp4'
        ff('-f','lavfi','-i','testsrc=size=64x36:rate=25','-t','0.4','-an','-c:v','libx264','-bf','0','-g','25','-keyint_min','25','-sc_threshold','0','-pix_fmt','yuv420p',separatefields_src)
        separatefields_args=''
        out=w/'separatefields.mkv';stats=native('transcode-lossless',separatefields_src,out,'--separatefields',separatefields_args)
        assert pixels(out)==pixels(separatefields_src,'separatefields')
        assert stats['video_frames']==20  # 0.4s@25fps (10 in) → separatefields emits 20
        record('separatefields matches FFmpeg pixel-exact',stats=stats)
        plan=native('plan',separatefields_src,'--separatefields',separatefields_args)
        assert plan['graph']=='separatefields'
        record('plan explains separatefields graph',plan=plan)
        weave_src=w/'weave-src.mp4'
        ff('-f','lavfi','-i','testsrc=size=64x36:rate=25','-t','0.4','-an','-c:v','libx264','-bf','0','-g','25','-keyint_min','25','-sc_threshold','0','-pix_fmt','yuv420p',weave_src)
        weave_args=''
        out=w/'weave.mkv';stats=native('transcode-lossless',weave_src,out,'--weave',weave_args)
        assert pixels(out)==pixels(weave_src,'weave')
        assert stats['video_frames']==5  # 0.4s@25fps (10 in) → weave emits 5 at 2x height
        record('weave matches FFmpeg pixel-exact',stats=stats)
        plan=native('plan',weave_src,'--weave',weave_args)
        assert plan['graph']=='weave'
        record('plan explains weave graph',plan=plan)
        doubleweave_src=w/'doubleweave-src.mp4'
        ff('-f','lavfi','-i','testsrc=size=64x36:rate=25','-t','0.4','-an','-c:v','libx264','-bf','0','-g','25','-keyint_min','25','-sc_threshold','0','-pix_fmt','yuv420p',doubleweave_src)
        doubleweave_args=''
        out=w/'doubleweave.mkv';stats=native('transcode-lossless',doubleweave_src,out,'--doubleweave',doubleweave_args)
        assert pixels(out)==pixels(doubleweave_src,'doubleweave')
        assert stats['video_frames']==9  # 0.4s@25fps (10 in) → doubleweave emits 9 at 2x height (matches FFmpeg)
        record('doubleweave matches FFmpeg pixel-exact',stats=stats)
        plan=native('plan',doubleweave_src,'--doubleweave',doubleweave_args)
        assert plan['graph']=='doubleweave'
        record('plan explains doubleweave graph',plan=plan)
        framepack_src=w/'framepack-src.mp4'
        ff('-f','lavfi','-i','testsrc=size=64x36:rate=25','-t','0.4','-an','-c:v','libx264','-bf','0','-g','25','-keyint_min','25','-sc_threshold','0','-pix_fmt','yuv420p',framepack_src)
        framepack_args='format=sbs'
        out=w/'framepack.mkv';stats=native('transcode-lossless',framepack_src,out,'--framepack',framepack_args)
        assert pixels(out)==framepack_pixels(framepack_src,framepack_args)
        assert stats['video_frames']==5  # 0.4s@25fps (10 in) → framepack sbs emits 5 at 2x width
        record('framepack matches FFmpeg pixel-exact',stats=stats)
        plan=native('plan',framepack_src,'--framepack',framepack_args)
        assert plan['graph']=='framepack=format=sbs'
        record('plan explains framepack graph',plan=plan)
        telecine_src=w/'telecine-src.mp4'
        ff('-f','lavfi','-i','testsrc=size=64x36:rate=25','-t','0.4','-an','-c:v','libx264','-bf','0','-g','25','-keyint_min','25','-sc_threshold','0','-pix_fmt','yuv420p',telecine_src)
        telecine_args='pattern=23'
        out=w/'telecine.mkv';stats=native('transcode-lossless',telecine_src,out,'--telecine',telecine_args)
        oracle=telecine_pixels(telecine_src,telecine_args)
        assert pixels(out)==oracle
        assert oracle!=pixels(telecine_src)
        assert stats['video_frames']==12  # 0.4s@25fps (10 in) → telecine pattern=23 emits 12
        record('telecine matches FFmpeg pixel-exact',stats=stats)
        plan=native('plan',telecine_src,'--telecine',telecine_args)
        assert plan['graph']=='telecine=pattern=23'
        record('plan explains telecine graph',plan=plan)
        pullup_src=w/'pullup-src.mp4'
        ff('-f','lavfi','-i','testsrc=size=64x36:rate=25','-t','0.4','-an','-c:v','libx264','-bf','0','-g','25','-keyint_min','25','-sc_threshold','0','-pix_fmt','yuv420p',pullup_src)
        pullup_args=''
        out=w/'pullup.mkv';stats=native('transcode-lossless',pullup_src,out,'--pullup',pullup_args)
        oracle=pullup_pixels(pullup_src,pullup_args)
        assert pixels(out)==oracle
        assert stats['video_frames']==8  # 0.4s@25fps (10 in) → pullup emits 8
        record('pullup matches FFmpeg pixel-exact',stats=stats)
        plan=native('plan',pullup_src,'--pullup',pullup_args)
        assert plan['graph']=='pullup'
        record('plan explains pullup graph',plan=plan)
        decimate_src=w/'decimate-src.mp4'
        ff('-f','lavfi','-i','testsrc=size=64x36:rate=25','-t','0.4','-an','-c:v','libx264','-bf','0','-g','25','-keyint_min','25','-sc_threshold','0','-pix_fmt','yuv420p',decimate_src)
        decimate_args='cycle=5'
        out=w/'decimate.mkv';stats=native('transcode-lossless',decimate_src,out,'--decimate',decimate_args)
        oracle=decimate_pixels(decimate_src,decimate_args)
        assert pixels(out)==oracle
        assert oracle!=pixels(decimate_src)
        assert stats['video_frames']==8  # 0.4s@25fps (10 in) → decimate cycle=5 emits 8
        record('decimate matches FFmpeg pixel-exact',stats=stats)
        plan=native('plan',decimate_src,'--decimate',decimate_args)
        assert plan['graph']=='decimate=cycle=5'
        record('plan explains decimate graph',plan=plan)
        mpdecimate_src=w/'mpdecimate-src.mp4'
        ff('-f','lavfi','-i','testsrc=size=64x36:rate=25','-t','0.4','-an','-c:v','libx264','-bf','0','-g','25','-keyint_min','25','-sc_threshold','0','-pix_fmt','yuv420p',mpdecimate_src)
        mpdecimate_args=''
        out=w/'mpdecimate.mkv';stats=native('transcode-lossless',mpdecimate_src,out,'--mpdecimate',mpdecimate_args)
        oracle=mpdecimate_pixels(mpdecimate_src,mpdecimate_args)
        assert pixels(out)==oracle
        assert oracle!=pixels(mpdecimate_src)
        assert stats['video_frames']==4  # 0.4s@25fps (10 in) → mpdecimate emits 4
        record('mpdecimate matches FFmpeg pixel-exact',stats=stats)
        plan=native('plan',mpdecimate_src,'--mpdecimate',mpdecimate_args)
        assert plan['graph']=='mpdecimate'
        record('plan explains mpdecimate graph',plan=plan)
        framestep_src=w/'framestep-src.mp4'
        ff('-f','lavfi','-i','testsrc=size=64x36:rate=25','-t','0.4','-an','-c:v','libx264','-bf','0','-g','25','-keyint_min','25','-sc_threshold','0','-pix_fmt','yuv420p',framestep_src)
        framestep_args='2'
        out=w/'framestep.mkv';stats=native('transcode-lossless',framestep_src,out,'--framestep',framestep_args)
        oracle=framestep_pixels(framestep_src,framestep_args)
        assert pixels(out)==oracle
        assert oracle!=pixels(framestep_src)
        assert stats['video_frames']==5  # 0.4s@25fps (10 in) → framestep=2 emits 5
        record('framestep matches FFmpeg pixel-exact',stats=stats)
        plan=native('plan',framestep_src,'--framestep',framestep_args)
        assert plan['graph']=='framestep=2'
        record('plan explains framestep graph',plan=plan)
        tile_src=w/'tile-src.mp4'
        ff('-f','lavfi','-i','testsrc=size=64x36:rate=25','-t','0.32','-an','-c:v','libx264','-bf','0','-g','25','-keyint_min','25','-sc_threshold','0','-pix_fmt','yuv420p',tile_src)
        tile_args='2x2'
        out=w/'tile.mkv';stats=native('transcode-lossless',tile_src,out,'--tile',tile_args)
        oracle=tile_pixels(tile_src,tile_args)
        assert pixels(out)==oracle
        assert oracle!=pixels(tile_src)
        assert stats['video_frames']==2  # 0.32s@25fps (8 in) → tile=2x2 emits 2 at 2x geometry
        record('tile matches FFmpeg pixel-exact',stats=stats)
        plan=native('plan',tile_src,'--tile',tile_args)
        assert plan['graph']=='tile=2x2'
        record('plan explains tile graph',plan=plan)
        untile_src=w/'untile-src.mp4'
        ff('-f','lavfi','-i','testsrc=size=64x36:rate=25','-t','0.32','-an','-c:v','libx264','-bf','0','-g','25','-keyint_min','25','-sc_threshold','0','-pix_fmt','yuv420p',untile_src)
        untile_args='2x2'
        out=w/'untile.mkv';stats=native('transcode-lossless',untile_src,out,'--untile',untile_args)
        oracle=untile_pixels(untile_src,untile_args)
        assert pixels(out)==oracle
        assert oracle!=pixels(untile_src)
        assert stats['video_frames']==32  # 0.32s@25fps (8 in) → untile=2x2 emits 32 at half geometry
        record('untile matches FFmpeg pixel-exact',stats=stats)
        plan=native('plan',untile_src,'--untile',untile_args)
        assert plan['graph']=='untile=2x2'
        record('plan explains untile graph',plan=plan)
        shuffleframes_src=w/'shuffleframes-src.mp4'
        ff('-f','lavfi','-i','testsrc=size=64x36:rate=25','-t','0.12','-an','-c:v','libx264','-bf','0','-g','25','-keyint_min','25','-sc_threshold','0','-pix_fmt','yuv420p',shuffleframes_src)
        shuffleframes_args='2 1 0'
        out=w/'shuffleframes.mkv';stats=native('transcode-lossless',shuffleframes_src,out,'--shuffleframes',shuffleframes_args)
        oracle=shuffleframes_pixels(shuffleframes_src,shuffleframes_args)
        assert pixels(out)==oracle
        assert oracle!=pixels(shuffleframes_src)
        assert stats['video_frames']==3  # 0.12s@25fps (3 in) → shuffleframes=2 1 0 emits 3 reordered
        record('shuffleframes matches FFmpeg pixel-exact',stats=stats)
        plan=native('plan',shuffleframes_src,'--shuffleframes',shuffleframes_args)
        assert plan['graph']=='shuffleframes=2 1 0'
        record('plan explains shuffleframes graph',plan=plan)
        reverse_src=w/'reverse-src.mp4'
        ff('-f','lavfi','-i','testsrc=size=64x36:rate=25','-t','0.2','-an','-c:v','libx264','-bf','0','-g','25','-keyint_min','25','-sc_threshold','0','-pix_fmt','yuv420p',reverse_src)
        reverse_args=''
        out=w/'reverse.mkv';stats=native('transcode-lossless',reverse_src,out,'--reverse',reverse_args)
        oracle=reverse_pixels(reverse_src,reverse_args)
        assert pixels(out)==oracle
        assert oracle!=pixels(reverse_src)
        assert stats['video_frames']==5  # 0.2s@25fps (5 in) → reverse emits 5 in reverse order
        record('reverse matches FFmpeg pixel-exact',stats=stats)
        plan=native('plan',reverse_src,'--reverse',reverse_args)
        assert plan['graph']=='reverse'
        record('plan explains reverse graph',plan=plan)
        loop_src=w/'loop-src.mp4'
        ff('-f','lavfi','-i','testsrc=size=64x36:rate=25','-t','0.2','-an','-c:v','libx264','-bf','0','-g','25','-keyint_min','25','-sc_threshold','0','-pix_fmt','yuv420p',loop_src)
        loop_args='1:size=2:start=0'
        out=w/'loop.mkv';stats=native('transcode-lossless',loop_src,out,'--loop',loop_args)
        oracle=loop_pixels(loop_src,loop_args)
        assert pixels(out)==oracle
        assert oracle!=pixels(loop_src)
        assert stats['video_frames']==7  # 0.2s@25fps (5 in) → loop=1:size=2:start=0 emits 7
        record('loop matches FFmpeg pixel-exact',stats=stats)
        plan=native('plan',loop_src,'--loop',loop_args)
        assert plan['graph']==f'loop={loop_args}'
        record('plan explains loop graph',plan=plan)
        thumbnail_src=w/'thumbnail-src.mp4'
        ff('-f','lavfi','-i','testsrc=size=64x36:rate=25','-t','0.24','-an','-c:v','libx264','-bf','0','-g','25','-keyint_min','25','-sc_threshold','0','-pix_fmt','yuv420p',thumbnail_src)
        thumbnail_args='n=3'
        out=w/'thumbnail.mkv';stats=native('transcode-lossless',thumbnail_src,out,'--thumbnail',thumbnail_args)
        oracle=thumbnail_pixels(thumbnail_src,thumbnail_args)
        assert pixels(out)==oracle
        assert oracle!=pixels(thumbnail_src)
        assert stats['video_frames']==2  # 0.24s@25fps (6 in) → thumbnail=n=3 emits 2
        record('thumbnail matches FFmpeg pixel-exact',stats=stats)
        plan=native('plan',thumbnail_src,'--thumbnail',thumbnail_args)
        assert plan['graph']==f'thumbnail={thumbnail_args}'
        record('plan explains thumbnail graph',plan=plan)
        interp_src=w/'interp.mp4'
        ff('-f','lavfi','-i','testsrc=size=128x72:rate=25','-t','0.4','-an','-c:v','libx264','-bf','0','-g','25','-keyint_min','25','-sc_threshold','0','-pix_fmt','yuv420p',interp_src)
        mi_args='mi_mode=blend:fps=50'
        out=w/'minterpolate.mkv';stats=native('transcode-lossless',interp_src,out,'--minterpolate',mi_args)
        assert pixels(out)==pixels(interp_src,f'minterpolate={mi_args}')
        assert stats['video_frames']==17  # 0.4s@25fps (10 in) → minterpolate blend@50 emits 17
        record('minterpolate matches FFmpeg pixel-exact',stats=stats)
        plan=native('plan',interp_src,'--minterpolate',mi_args)
        assert plan['graph']==f'minterpolate={mi_args}'
        record('plan explains minterpolate graph',plan=plan)
        amplify_src=w/'amplify-src.mp4'
        ff('-f','lavfi','-i','testsrc=size=128x72:rate=25','-t','0.4','-an','-c:v','libx264','-bf','0','-g','25','-keyint_min','25','-sc_threshold','0','-pix_fmt','yuv420p',amplify_src)
        amp_args='radius=2:factor=2'
        out=w/'amplify.mkv';stats=native('transcode-lossless',amplify_src,out,'--amplify',amp_args)
        assert pixels(out)==pixels(amplify_src,f'amplify={amp_args}')
        assert stats['video_frames']==5  # 0.4s@25fps (10 in) → amplify emits 5
        record('amplify matches FFmpeg pixel-exact',stats=stats)
        plan=native('plan',amplify_src,'--amplify',amp_args)
        assert plan['graph']==f'amplify={amp_args}'
        record('plan explains amplify graph',plan=plan)
        pseudocolor_src=w/'pseudocolor-src.mp4'
        ff('-f','lavfi','-i','testsrc=size=128x72:rate=25','-t','0.4','-an','-c:v','libx264','-bf','0','-g','25','-keyint_min','25','-sc_threshold','0','-pix_fmt','yuv420p',pseudocolor_src)
        pc_args='preset=magma'
        out=w/'pseudocolor.mkv';stats=native('transcode-lossless',pseudocolor_src,out,'--pseudocolor',pc_args)
        assert pixels(out)==pixels(pseudocolor_src,f'pseudocolor={pc_args}')
        assert stats['video_frames']==10
        record('pseudocolor matches FFmpeg pixel-exact',stats=stats)
        plan=native('plan',pseudocolor_src,'--pseudocolor',pc_args)
        assert plan['graph']==f'pseudocolor={pc_args}'
        record('plan explains pseudocolor graph',plan=plan)
        loud=w/'loud.wav';ff('-f','lavfi','-i','sine=frequency=997:sample_rate=48000:duration=2','-ac','2',loud)
        stats=native('loudness',loud)
        assert stats['sample_rate']==48000 and stats['channels']==2 and stats['sample_frames']>0
        ref=run(['ffmpeg','-nostdin','-i',loud,'-af','ebur128=peak=true','-f','null','-'])
        text=ref.stderr.decode(errors='replace')
        import re
        summary=text.rsplit('Summary:',1)[-1]
        m=re.search(r'I:\s+(-?\d+\.\d+)\s+LUFS',summary)
        assert m,text
        assert abs(stats['integrated_lufs']-float(m.group(1)))<0.15
        tp=re.search(r'Peak:\s+(-?\d+\.\d+)\s+dBFS',summary)
        assert tp and abs(stats['true_peak_dbfs']-float(tp.group(1)))<0.15
        record('loudness ebur128 matches FFmpeg integrated/true-peak',stats=stats)
        ln_args='I=-16:TP=-1.5:LRA=11'
        out=w/'loudnorm-fvid.wav';stats=native('loudnorm',loud,out,'--loudnorm-args',ln_args)
        assert stats['args']==ln_args and stats['sample_rate']==48000 and stats['channels']==2
        ref=w/'loudnorm-ff.wav'
        run(['ffmpeg','-nostdin','-y','-v','error','-i',loud,'-af',f'loudnorm={ln_args},aformat=sample_fmts=flt','-c:a','pcm_f32le',ref])
        expected=ff('-i',ref,'-c:a','pcm_f32le','-f','f32le','-').stdout
        actual=ff('-i',out,'-c:a','pcm_f32le','-f','f32le','-').stdout
        assert actual==expected and stats['sample_frames']>0
        record('loudnorm apply matches FFmpeg loudnorm=+aformat=flt sample-exact',stats=stats)
        out=w/'loudnorm-dual-fvid.wav';stats=native('loudnorm',loud,out,'--loudnorm-args',ln_args,'--dual-pass')
        assert stats.get('dual_pass') is True and 'measured_I=' in stats['args'] and 'linear=true' in stats['args']
        p1=run(['ffmpeg','-nostdin','-i',loud,'-af',f'loudnorm={ln_args}:print_format=json','-f','null','-'])
        text=p1.stderr.decode(errors='replace'); start=text.rfind('{'); end=text.find('}',start); assert start>=0 and end>start
        measured=json.loads(text[start:end+1])
        pass2=f"{ln_args}:measured_I={measured['input_i']}:measured_TP={measured['input_tp']}:measured_LRA={measured['input_lra']}:measured_thresh={measured['input_thresh']}:offset={measured['target_offset']}:linear=true"
        assert stats['args']==pass2
        ref=w/'loudnorm-dual-ff.wav'
        run(['ffmpeg','-nostdin','-y','-v','error','-i',loud,'-af',f'loudnorm={pass2},aformat=sample_fmts=flt','-c:a','pcm_f32le',ref])
        expected=ff('-i',ref,'-c:a','pcm_f32le','-f','f32le','-').stdout
        actual=ff('-i',out,'-c:a','pcm_f32le','-f','f32le','-').stdout
        assert actual==expected and stats['sample_frames']>0
        record('loudnorm --dual-pass matches FFmpeg two-pass measured sample-exact',stats=stats)
        plan=native('plan','loudness',loud)
        assert plan['command']=='loudness' and plan['graph']=='ebur128=peak=true'
        record('plan loudness subcommand explains ebur128',plan=plan)
        plan=native('plan','loudnorm',loud,'--loudnorm-args',ln_args)
        assert plan['command']=='loudnorm' and plan['graph']==f'loudnorm={ln_args},aformat=sample_fmts=flt'
        record('plan loudnorm subcommand explains apply graph',plan=plan)
        plan=native('plan','loudnorm',loud,'--loudnorm-args',ln_args,'--dual-pass')
        assert plan['command']=='loudnorm' and 'linear=true' in plan['graph'] and any(s['action']=='analyze' for s in plan['steps'])
        record('plan loudnorm --dual-pass explains measure+apply',plan=plan)
        subtitled=w/'subtitled.mkv'
        ff('-i',source,'-f','srt','-i',subtitles,'-map','0:v:0','-map','1:s:0','-metadata:s:s:0','language=eng','-disposition:s:0','default','-c','copy',subtitled)
        subtitle_info=native('probe',subtitled)
        subtitle_stream=subtitle_info['streams'][1]
        assert subtitle_stream['media_type']=='subtitle' and subtitle_stream['codec']=='subrip'
        assert subtitle_stream['metadata']['language']=='eng' and subtitle_stream['disposition']!=0
        out=w/'remux-subtitles.mkv';native('remux',subtitled,out)
        assert packets(out,'s:0')==packets(subtitled,'s:0')
        assert native('probe',out)['streams'][1]['metadata']['language']=='eng'
        record('remux preserves subtitle packets, timestamps, language and disposition')
        out=w/'meta-stream.mkv';stats=native('remux',subtitled,out,'--stream-metadata','1:language=rus')
        assert native('probe',out)['streams'][1]['metadata']['language']=='rus'
        assert packets(out,'s:0')==packets(subtitled,'s:0')
        record('remux sets stream language metadata',stats=stats)
        out=w/'meta-stream-delete.mkv';stats=native('remux',subtitled,out,'--stream-metadata-delete','1:language')
        assert 'language' not in native('probe',out)['streams'][1]['metadata']
        record('remux deletes stream metadata keys',stats=stats)
        out=w/'lossless-subtitles.mkv';native('transcode-lossless',subtitled,out)
        assert packets(out,'s:0')==packets(subtitled,'s:0')
        assert native('probe',out)['streams'][1]['disposition']==subtitle_stream['disposition']
        record('lossless video transcode maps subtitle stream without decoding')
        for start,end,expected in [('0.12','0.52',[('0.000000','0.400000')]),('0.52','1.52',[('0.000000','0.480000')]),('1','2',[])]:
            out=w/'chapter-trim.mkv';native('transcode-lossless',chaptered,out,'--from',start,'--to',end)
            chapters=json.loads(run(['ffprobe','-v','error','-show_chapters','-of','json',out]).stdout).get('chapters',[])
            assert [(c['start_time'],c['end_time']) for c in chapters]==expected
            assert all(c['tags']['title']=='First' for c in chapters)
            assert pixels(out)==pixels(chaptered,f'trim=start={start}:end={end}')
            record(f'lossless interval retimes chapters {start}:{end}');out.unlink()
        metadata.write_text(';FFMETADATA1\n[CHAPTER]\nTIMEBASE=1/1000\nSTART=0\nEND=400\ntitle=Before\n[CHAPTER]\nTIMEBASE=1/1000\nSTART=400\nEND=800\ntitle=Middle\n[CHAPTER]\nTIMEBASE=1/1000\nSTART=800\nEND=1600\ntitle=After\n')
        multi=w/'multi-chapters.mkv';ff('-i',source,'-i',metadata,'-map','0','-map_metadata','1','-map_chapters','1','-c','copy',multi)
        out=w/'multi-trim.mkv';native('transcode-lossless',multi,out,'--from','0.52','--to','1.2','--seek')
        chapters=json.loads(run(['ffprobe','-v','error','-show_chapters','-of','json',out]).stdout)['chapters']
        assert [(c['tags']['title'],c['start_time'],c['end_time']) for c in chapters]==[('Middle','0.000000','0.280000'),('After','0.280000','0.680000')]
        assert pixels(out)==pixels(multi,'trim=start=0.52:end=1.2')
        record('chapter compaction preserves subsequent metadata with seek')
        audio_chapters=w/'audio-chapters.mkv';ff('-i',aac,'-i',metadata,'-map','0:a:0','-map_chapters','1','-c','copy',audio_chapters)
        for name,args in [
            ('reject unqualified audio chapter mapping',['decode-audio',audio_chapters]),
            ('reject video in decode-audio',['decode-audio',source]),
            ('reject multiple streams in decode-audio',['decode-audio',aac]),
            ('reject invalid sample rate',['decode-audio',aac,'--rate','100']),
            ('reject invalid channels',['decode-audio',stereo,'--channels','0']),
            ('reject invalid volume',['decode-audio',aac,'--volume','-1']),
            ('reject volume on integer PCM',['decode-audio',av,'--streams','1','--volume','0.5']),
            ('reject rate on remux',['remux',source,'--rate','44100']),
            ('reject channels on remux',['remux',source,'--channels','1']),
            ('reject mix-audio one input',['mix-audio',w/'bad.wav',mix_a]),
            ('reject mix-audio rate mismatch',['mix-audio',w/'bad2.wav',mix_a,w/'mix-44k.m4a']),
            ('reject mix-audio excess weights',['mix-audio',w/'bad-w.wav',mix_a,'--weights','1,2,3']),
            ('reject merge-audio one input',['merge-audio',w/'bad-merge.wav',merge_l]),
            ('reject merge-audio rate mismatch',['merge-audio',w/'bad-merge2.wav',merge_l,w/'mix-44k.m4a']),
            ('reject absent encoder',['transcode',source]),
            ('reject unavailable encoder',['transcode',source,'--encoder','fvid-no-such-encoder']),
            ('reject unused encoder option',['transcode',source,'--encoder','libx264','--encoder-option','fvid-typo=1']),
            ('reject encoder override on lossless command',['transcode-lossless',source,'--encoder','libx264']),
            ('reject video in trim-pcm',['trim-pcm',av,'--from','0','--to','1']),
            ('reject compressed trim-pcm',['trim-pcm',aac,'--streams','1','--from','0','--to','1']),
            ('reject seek without interval',['transcode-lossless',bframes,'--seek']),
            ('reject empty lossless interval',['transcode-lossless',bframes,'--from','3','--to','4']),
            ('reject reversed lossless interval',['transcode-lossless',bframes,'--from','1','--to','0']),
            ('reject incomplete lossless interval',['transcode-lossless',bframes,'--from','1']),
            ('reject unqualified chapter trim',['trim',chaptered,'--from','0','--to','1']),
            ('reject non-IDR boundary',['trim',source,'--from','0.04','--to','1']),
            ('reject packet split',['trim',source,'--from','0.01','--to','1']),
            ('reject mid-GOP start without packet boundary',['trim',seek_source,'--from','1.13','--to','2']),
            ('reject B-frame trim without keyframe end',['trim',bframes,'--from','0','--to','1']),
            ('reject incompatible concat',['concat',source,av]),
            ('reject invalid crop',['crop-lossless',source,'--crop','127:0:64:48']),
            ('reject odd scale',['transcode-lossless',source,'--scale','63:36']),
            ('reject unknown pix-fmt',['transcode-lossless',source,'--pix-fmt','not_a_real_fmt']),
            ('reject invalid colorspace args',['transcode-lossless',source,'--colorspace','bt709;rm']),
            ('reject invalid loudnorm args',['loudnorm',loud,str(w/'ln-bad.wav'),'--loudnorm-args','I=-16;rm']),
            ('reject tonemap without pix-fmt',['transcode-lossless',source,'--tonemap','tonemap=hable']),
            ('reject invalid tonemap args',['transcode-lossless',source,'--tonemap','hable;rm','--pix-fmt','yuv420p']),
            ('reject zscale without pix-fmt',['transcode-lossless',source,'--zscale','matrix=bt709']),
            ('reject invalid zscale args',['transcode-lossless',source,'--zscale','matrix=bt709;rm','--pix-fmt','yuv420p']),
            ('reject invalid yadif args',['transcode-lossless',source,'--yadif','mode=0;rm']),
            ('reject invalid bwdif args',['transcode-lossless',source,'--bwdif','mode=0;rm']),
            ('reject invalid w3fdif args',['transcode-lossless',source,'--w3fdif','mode=0;rm']),
            ('reject invalid tblend args',['transcode-lossless',source,'--tblend','all_mode=average;rm']),
            ('reject invalid tmix args',['transcode-lossless',source,'--tmix','frames=3;rm']),
            ('reject invalid hqdn3d args',['transcode-lossless',source,'--hqdn3d','4:3;rm']),
            ('reject invalid fps args',['transcode-lossless',source,'--fps','12;rm']),
            ('reject invalid gblur args',['transcode-lossless',source,'--gblur','sigma=1;rm']),
            ('reject invalid eq args',['transcode-lossless',source,'--eq','brightness=0;rm']),
            ('reject invalid unsharp args',['transcode-lossless',source,'--unsharp','5:5:1;rm']),
            ('reject invalid hue args',['transcode-lossless',source,'--hue','h=45;rm']),
            ('reject invalid boxblur args',['transcode-lossless',source,'--boxblur','2;rm']),
            ('reject invalid avgblur args',['transcode-lossless',source,'--avgblur','sizeX=5;rm']),
            ('reject invalid negate args',['transcode-lossless',source,'--negate','2']),
            ('reject invalid edgedetect args',['transcode-lossless',source,'--edgedetect','mode=wires;rm']),
            ('reject invalid sobel args',['transcode-lossless',source,'--sobel','scale=2;rm']),
            ('reject invalid prewitt args',['transcode-lossless',source,'--prewitt','scale=2;rm']),
            ('reject invalid roberts args',['transcode-lossless',source,'--roberts','scale=2;rm']),
            ('reject invalid kirsch args',['transcode-lossless',source,'--kirsch','scale=2;rm']),
            ('reject invalid scharr args',['transcode-lossless',source,'--scharr','scale=2;rm']),
            ('reject invalid atadenoise args',['transcode-lossless',source,'--atadenoise','0a=0.02;rm']),
            ('reject invalid owdenoise args',['transcode-lossless',source,'--owdenoise','depth=8;rm']),
            ('reject invalid vaguedenoiser args',['transcode-lossless',source,'--vaguedenoiser','threshold=3;rm']),
            ('reject invalid nlmeans args',['transcode-lossless',source,'--nlmeans','s=1;rm']),
            ('reject invalid bm3d args',['transcode-lossless',source,'--bm3d','sigma=3;rm']),
            ('reject invalid dctdnoiz args',['transcode-lossless',source,'--dctdnoiz','s=3;rm']),
            ('reject invalid fftdnoiz args',['transcode-lossless',source,'--fftdnoiz','sigma=1;rm']),
            ('reject invalid smartblur args',['transcode-lossless',source,'--smartblur','lr=1;rm']),
            ('reject invalid sab args',['transcode-lossless',source,'--sab','lr=2;rm']),
            ('reject invalid bilateral args',['transcode-lossless',source,'--bilateral','sigmaS=0.1;rm']),
            ('reject invalid cas args',['transcode-lossless',source,'--cas','strength=1;rm']),
            ('reject invalid vignette args',['transcode-lossless',source,'--vignette','angle=1;rm']),
            ('reject invalid curves args',['transcode-lossless',source,'--curves','preset=1;rm']),
            ('reject invalid colorbalance args',['transcode-lossless',source,'--colorbalance','rs=1;rm']),
            ('reject invalid colorlevels args',['transcode-lossless',source,'--colorlevels','rimin=1;rm']),
            ('reject invalid colorchannelmixer args',['transcode-lossless',source,'--colorchannelmixer','rr=1;rm']),
            ('reject invalid deflicker args',['transcode-lossless',source,'--deflicker','mode=am;rm']),
            ('reject invalid photosensitivity args',['transcode-lossless',source,'--photosensitivity','f=5;rm']),
            ('reject invalid monochrome args',['transcode-lossless',source,'--monochrome','cb=0.2;rm']),
            ('reject invalid grayworld args',['transcode-lossless',source,'--grayworld','0;rm']),
            ('reject invalid drawbox args',['transcode-lossless',source,'--drawbox','x=10;rm']),
            ('reject invalid drawgrid args',['transcode-lossless',source,'--drawgrid','w=16;rm']),
            ('reject invalid lagfun args',['transcode-lossless',source,'--lagfun','decay=0.95;rm']),
            ('reject invalid amplify args',['transcode-lossless',source,'--amplify','radius=2;rm']),
            ('reject invalid bitplanenoise args',['transcode-lossless',source,'--bitplanenoise','bitplane=1;rm']),
            ('reject invalid deband args',['transcode-lossless',source,'--deband','1thr=0.02;rm']),
            ('reject invalid gradfun args',['transcode-lossless',source,'--gradfun','strength=1.2;rm']),
            ('reject invalid lenscorrection args',['transcode-lossless',source,'--lenscorrection','k1=-0.1;rm']),
            ('reject invalid pixelize args',['transcode-lossless',source,'--pixelize','width=8;rm']),
            ('reject invalid removegrain args',['transcode-lossless',source,'--removegrain','m0=1;rm']),
            ('reject invalid yaepblur args',['transcode-lossless',source,'--yaepblur','r=3;rm']),
            ('reject invalid vibrance args',['transcode-lossless',source,'--vibrance','intensity=0.3;rm']),
            ('reject invalid dilation args',['transcode-lossless',source,'--dilation','threshold0=10;rm']),
            ('reject invalid erosion args',['transcode-lossless',source,'--erosion','threshold0=10;rm']),
            ('reject invalid colorize args',['transcode-lossless',source,'--colorize','hue=120;rm']),
            ('reject invalid exposure args',['transcode-lossless',source,'--exposure','exposure=0.5;rm']),
            ('reject invalid chromashift args',['transcode-lossless',source,'--chromashift','cbh=4;rm']),
            ('reject invalid colorcontrast args',['transcode-lossless',source,'--colorcontrast','rc=0.1;rm']),
            ('reject invalid colorcorrect args',['transcode-lossless',source,'--colorcorrect','rl=0.1;rm']),
            ('reject invalid histeq args',['transcode-lossless',source,'--histeq','strength=0.2;rm']),
            ('reject invalid shuffleplanes args',['transcode-lossless',source,'--shuffleplanes','map0=0;rm']),
            ('reject invalid lutyuv args',['transcode-lossless',source,'--lutyuv','y=val*0.8;rm']),
            ('reject invalid colorhold args',['transcode-lossless',source,'--colorhold','similarity=0.2;rm']),
            ('reject invalid fade args',['transcode-lossless',source,'--fade','t=in;s=0;rm']),
            ('reject invalid perspective args',['transcode-lossless',source,'--perspective','x0=0;rm']),
            ('reject invalid lumakey args',['transcode-lossless',source,'--lumakey','threshold=0.1;rm']),
            ('reject invalid chromakey args',['transcode-lossless',source,'--chromakey','similarity=0.3;rm']),
            ('reject invalid colorkey args',['transcode-lossless',source,'--colorkey','similarity=0.1;rm']),
            ('reject invalid despill args',['transcode-lossless',source,'--despill','type=green;rm']),
            ('reject invalid selectivecolor args',['transcode-lossless',source,'--selectivecolor','reds=0.2;rm']),
            ('reject invalid stereo3d args',['transcode-lossless',source,'--stereo3d','sbsl:abl;rm']),
            ('reject invalid field args',['transcode-lossless',source,'--field','bottom;rm']),
            ('reject invalid hqx args',['transcode-lossless',source,'--hqx','n=2;rm']),
            ('reject invalid xbr args',['transcode-lossless',source,'--xbr','n=2;rm']),
            ('reject invalid il args',['transcode-lossless',source,'--il','l=d;rm']),
            ('reject invalid super2xsai args',['transcode-lossless',source,'--super2xsai',';rm']),
            ('reject invalid kerndeint args',['transcode-lossless',source,'--kerndeint','thresh=10;rm']),
            ('reject invalid phase args',['transcode-lossless',source,'--phase','mode=t;rm']),
            ('reject invalid estdif args',['transcode-lossless',source,'--estdif','mode=frame;rm']),
            ('reject invalid tinterlace args',['transcode-lossless',source,'--tinterlace','mode=interleave_top;rm']),
            ('reject invalid separatefields args',['transcode-lossless',source,'--separatefields',';rm']),
            ('reject invalid weave args',['transcode-lossless',source,'--weave',';rm']),
            ('reject invalid doubleweave args',['transcode-lossless',source,'--doubleweave',';rm']),
            ('reject invalid framepack args',['transcode-lossless',source,'--framepack',';rm']),
            ('reject invalid telecine args',['transcode-lossless',source,'--telecine',';rm']),
            ('reject invalid pullup args',['transcode-lossless',source,'--pullup',';rm']),
            ('reject invalid decimate args',['transcode-lossless',source,'--decimate',';rm']),
            ('reject invalid mpdecimate args',['transcode-lossless',source,'--mpdecimate',';rm']),
            ('reject invalid framestep args',['transcode-lossless',source,'--framestep',';rm']),
            ('reject invalid tile args',['transcode-lossless',source,'--tile',';rm']),
            ('reject invalid untile args',['transcode-lossless',source,'--untile',';rm']),
            ('reject invalid shuffleframes args',['transcode-lossless',source,'--shuffleframes',';rm']),
            ('reject invalid reverse args',['transcode-lossless',source,'--reverse',';rm']),
            ('reject invalid loop args',['transcode-lossless',source,'--loop',';rm']),
            ('reject invalid thumbnail args',['transcode-lossless',source,'--thumbnail',';rm']),
            ('reject invalid pseudocolor args',['transcode-lossless',source,'--pseudocolor','preset=magma;rm']),
            ('reject invalid minterpolate args',['transcode-lossless',source,'--minterpolate','mi_mode=blend;rm']),
            ('reject invalid transpose',['transcode-lossless',source,'--transpose','upside']),
            ('reject transpose on crop-lossless',['crop-lossless',source,'--crop','2:2:64:48','--transpose','clock']),
            ('reject invalid rotate',['transcode-lossless',source,'--rotate','not-a-number']),
            ('reject rotate on crop-lossless',['crop-lossless',source,'--crop','2:2:64:48','--rotate','45']),
            ('reject undersized pad',['transcode-lossless',source,'--pad','64:48:0:0']),
            ('reject pad on crop-lossless',['crop-lossless',source,'--crop','2:2:64:48','--pad','192:108:0:0']),
            ('reject metadata on trim',['trim',source,'--from','1','--to','2','--metadata','title=X']),
            ('reject nonexistent stream metadata',['remux',source,'--stream-metadata','9:language=eng']),
            ('reject video in convert-subtitles',['convert-subtitles',source,'--codec','ass']),
            ('reject burn-subtitles without --subs',['burn-subtitles',source]),
            ('reject missing burn subtitle file',['burn-subtitles',source,'--subs',str(w/'missing.srt')]),
            ('reject missing overlay file',['overlay',source,str(w/'missing.mp4'),str(w/'out.mkv')]),
            ('reject subs on remux',['remux',source,'--subs',str(burn_subs)]),
            ('reject nonexistent stream',['remux',source,'--streams','9']),
            ('reject packet budget',['remux',source,'--max-packets','1']),
            ('reject controlled memory budget',['remux',source,'--max-memory-mib','1']),
            ('reject rss budget',['remux',source,'--max-rss-mib','1']),
        ]:
            target=w/'rejected.mkv'
            command=[BINARY,'media',args[0],target,*args[1:]]if args[0]=='concat'else[BINARY,'media',args[0],args[1],target,*args[2:]]
            result=subprocess.run([str(v)for v in command],capture_output=True,timeout=60)
            assert result.returncode and not target.exists() and not list(w.glob('.fvid-media-*.tmp')), (name,result.stderr)
            record(name,error=result.stderr.decode())
        result=subprocess.run([str(BINARY),'media','remux',str(source),str(copied)],capture_output=True)
        assert result.returncode and hashes(source)==hashes(copied);record('existing output preserved')
        budget_out=w/'budget.mkv'
        result=subprocess.run([str(BINARY),'media','remux',str(source),str(budget_out),'--max-packets','3'],capture_output=True)
        assert result.returncode and not budget_out.exists() and b'packet budget exceeded' in result.stderr
        record('packet budget aborts without publishing output')
        mem_out=w/'mem-budget.mkv'
        result=subprocess.run([str(BINARY),'media','remux',str(source),str(mem_out),'--max-memory-mib','1'],capture_output=True)
        assert result.returncode and not mem_out.exists() and b'controlled memory budget exceeded' in result.stderr
        record('controlled memory budget aborts without publishing output')
        rss_out=w/'rss-budget.mkv'
        result=subprocess.run([str(BINARY),'media','remux',str(source),str(rss_out),'--max-rss-mib','1'],capture_output=True)
        assert result.returncode and not rss_out.exists() and b'rss budget exceeded' in result.stderr
        record('rss budget aborts without publishing output')
        big=w/'budget4k.mp4'
        run(['ffmpeg','-nostdin','-y','-v','error','-f','lavfi','-i','testsrc2=size=1920x1080:rate=30','-frames:v','8','-pix_fmt','yuv420p','-c:v','libx264','-bf','2','-g','8','-keyint_min','8','-an',big])
        big_out=w/'budget4k.mkv'
        result=subprocess.run([str(BINARY),'media','transcode-lossless',str(big),str(big_out),'--max-memory-mib','1'],capture_output=True)
        assert result.returncode and not big_out.exists() and b'controlled memory budget exceeded' in result.stderr
        record('decode DPB estimate rejects undersized max-memory-mib')
    capabilities=native('capabilities')
    report=dict(created_utc=datetime.datetime.now(datetime.timezone.utc).isoformat(),status='passed',checks=checks,
                capability_counts={k:len(v)for k,v in capabilities.items()if isinstance(v,list)},
                capability_note='Inventory of owned FVid components; not proof of all profiles or workflows.',
                library_version=capabilities['library_version'],binary_sha256=hashlib.sha256(BINARY.read_bytes()).hexdigest(),
                source_sha256={str(p.relative_to(ROOT)):hashlib.sha256(p.read_bytes()).hexdigest() for p in [*ROOT.joinpath('src').rglob('*.rs'),*ROOT.joinpath('crates/fvid-media/src').rglob('*.rs'),ROOT/'crates/fvid-media/build.rs',ROOT/'Cargo.toml',ROOT/'Cargo.lock',ROOT/'crates/fvid-media/Cargo.toml',pathlib.Path(__file__)]})
    (ROOT/'benchmarks/media-validation.json').write_text(json.dumps(report,indent=2)+'\n')
    print(f'{len(checks)} checks passed',flush=True)
if __name__=='__main__':main()
