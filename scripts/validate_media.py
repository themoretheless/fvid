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
def audio(path):return ff('-i',path,'-map','0:a:0','-vn','-c:a','pcm_s16le','-f','s16le','-').stdout

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
        info=native('probe',source);assert info['streams'][0]['codec']=='h264';record('probe H264 MP4',info=info)
        copied=w/'remux.mkv';stats=native('remux',source,copied);assert hashes(source)==hashes(copied);assert pixels(source)==pixels(copied);record('remux MP4 to MKV packet and decoded equality',stats=stats)
        trimmed=w/'trim.mp4';stats=native('trim',source,trimmed,'--from','1','--to','2');assert hashes(trimmed)==hashes(source)[25:];assert pixels(trimmed)==pixels(source)[25*128*72*3//2:];record('exact IDR trim',stats=stats)
        concatenated=w/'concat.mp4';stats=native('concat',concatenated,source,source);assert hashes(concatenated)==hashes(source)*2;assert pixels(concatenated)==pixels(source)*2;record('concat H264 two segments',stats=stats)
        out=w/'crop.mkv';stats=native('crop-lossless',source,out,'--crop','2:2:64:48');assert pixels(out)==pixels(source,'crop=64:48:2:2:exact=1');record('H264 decode -> borrowed crop -> FFV1',stats=stats)
        bframes=w/'bframes.mp4';ff('-f','lavfi','-i','testsrc2=size=128x72:rate=25','-t','2','-c:v','libx264','-bf','3',bframes)
        out=w/'bframes-crop.mkv';stats=native('crop-lossless',bframes,out,'--crop','2:2:64:48');assert pixels(out)==pixels(bframes,'crop=64:48:2:2:exact=1');record('B-frame decoder drain and lossless crop',stats=stats)
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
        out=w/'av-crop.mkv';stats=native('crop-lossless',av,out,'--crop','2:2:64:48');assert pixels(out)==pixels(av,'crop=64:48:2:2:exact=1');assert audio(out)==audio(av);record('lossless crop preserves PCM audio samples',stats=stats)
        out=w/'av-sample-trim.mkv';stats=native('transcode-lossless',av,out,'--from','0.125','--to','0.525')
        assert pixels(out)==pixels(av,'trim=start=0.125:end=0.525')
        assert audio(out)==audio(av)[6000*2:25200*2]
        assert stats['trimmed_audio_sample_frames']==19200
        record('lossless interval cuts inside PCM packets without sample changes',stats=stats)
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
        out=w/'av-trim.mkv';stats=native('trim',av,out,'--from','1','--to','2');assert pixels(out)==pixels(av)[25*128*72*3//2:];assert audio(out)==audio(av)[48000*2:];record('strict video+audio aligned trim',stats=stats)
        out=w/'av-concat.mkv';stats=native('concat',out,av,av);assert pixels(out)==pixels(av)*2;assert audio(out)==audio(av)*2;record('strict video+audio concat',stats=stats)
        high=w/'high.mkv';ff('-f','lavfi','-i','testsrc2=size=128x72:rate=25','-t','0.4','-pix_fmt','yuv420p10le','-c:v','ffv1','-level','3',high)
        out=w/'high-crop.mkv';stats=native('crop-lossless',high,out,'--crop','2:2:64:48');assert pixels(out,pixfmt='yuv420p10le')==pixels(high,'crop=64:48:2:2:exact=1','yuv420p10le');record('10-bit crop preserves samples',stats=stats)
        for name,src,fmt,flags,reference in [
            ('identity lossless B-frame transcode',bframes,'yuv420p',[],None),
            ('vertical flip with copied PCM',av,'yuv420p',['--vflip'],'vflip'),
            ('crop then vertical flip 10-bit',high,'yuv420p10le',['--crop','2:2:64:48','--vflip'],'crop=64:48:2:2:exact=1,vflip'),
        ]:
            out=w/'transform.mkv';stats=native('transcode-lossless',src,out,*flags)
            assert pixels(out,pixfmt=fmt)==pixels(src,reference,fmt)
            if src==av:assert audio(out)==audio(src)
            record(name,stats=stats);out.unlink()
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
        for encoder,extension,options in [('libx264','mp4',['--encoder-option','crf=0','--encoder-option','preset=fast']),('libvpx-vp9','webm',['--encoder-option','lossless=1','--encoder-option','deadline=good','--encoder-option','cpu-used=4'])]:
            out=w/f'explicit-{encoder}.{extension}';stats=native('transcode',source,out,'--encoder',encoder,*options,'--crop','2:2:64:48','--hflip')
            assert pixels(out)==pixels(source,'crop=64:48:2:2:exact=1,hflip')
            assert stats['encoder']==encoder and stats['video_frames']==50
            record(f'explicit {encoder} lossless mode with crop/hflip',stats=stats)
        out=w/'lossy.mp4';stats=native('transcode',source,out,'--encoder','libx264','--encoder-option','crf=28')
        info=native('probe',out);assert info['streams'][0]['codec']=='h264'
        assert len(pixels(out))==len(pixels(source)) and pixels(out)!=pixels(source)
        record('explicit lossy H264 preserves frame count and geometry',stats=stats)
        aac=w/'aac.mp4';ff('-f','lavfi','-i','testsrc2=size=128x72:rate=25','-f','lavfi','-i','sine=frequency=733:sample_rate=48000','-t','2','-c:v','libx264','-c:a','aac',aac)
        out=w/'aac-remux.mp4';stats=native('remux',aac,out);assert hashes(aac,'a:0')==hashes(out,'a:0');assert audio(aac)==audio(out);record('AAC MP4 remux preserves decoded samples',stats=stats)
        out=w/'aac-crop.mkv';stats=native('crop-lossless',aac,out,'--crop','2:2:64:48');assert audio(aac)==audio(out);assert hashes(aac,'a:0')==hashes(out,'a:0');record('AAC crop preserves decoded samples',stats=stats)
        out=w/'aac-decoded.wav';stats=native('decode-audio',aac,out,'--streams','1')
        expected=ff('-i',aac,'-map','0:a:0','-c:a','pcm_f32le','-f','f32le','-').stdout
        actual=ff('-i',out,'-c:a','pcm_f32le','-f','f32le','-').stdout
        assert actual==expected and stats['sample_format']=='flt'
        assert stats['channels']==1 and stats['planar_interleave_bytes']==0
        record('AAC decode to float PCM preserves every decoded sample',stats=stats)
        stereo=w/'stereo-aac.m4a';ff('-f','lavfi','-i','aevalsrc=sin(2*PI*997*t)|sin(2*PI*1733*t):s=48000','-t','1','-c:a','aac',stereo)
        out=w/'stereo-decoded.wav';stats=native('decode-audio',stereo,out)
        expected=ff('-i',stereo,'-c:a','pcm_f32le','-f','f32le','-').stdout
        actual=ff('-i',out,'-c:a','pcm_f32le','-f','f32le','-').stdout
        assert actual==expected and stats['planar_interleave_bytes']==len(actual)
        record('stereo AAC distinct channels preserve bits through pooled SIMD interleave',stats=stats)
        for codec,extension,pcm,raw in [('libmp3lame','mp3','pcm_f32le','f32le'),('flac','flac','pcm_s16le','s16le')]:
            src=w/f'audio.{extension}';ff('-i',av,'-map','0:a:0','-c:a',codec,src)
            out=w/f'decoded-{extension}.wav';stats=native('decode-audio',src,out)
            expected=ff('-i',src,'-c:a',pcm,'-f',raw,'-').stdout
            actual=ff('-i',out,'-c:a',pcm,'-f',raw,'-').stdout
            assert actual==expected
            record(f'{extension} decode preserves samples and codec delay handling',stats=stats)
        metadata=w/'chapters.txt';metadata.write_text(';FFMETADATA1\ntitle=Fixture\n[CHAPTER]\nTIMEBASE=1/1000\nSTART=0\nEND=1000\ntitle=First\n')
        chaptered=w/'chaptered.mkv';ff('-i',source,'-i',metadata,'-map','0','-map_metadata','1','-map_chapters','1','-c','copy',chaptered)
        for mode,flags in [('remux',[]),('crop-lossless',['--crop','2:2:64:48'])]:
            out=w/f'chapters-{mode}.mkv';native(mode,chaptered,out,*flags)
            chapters=json.loads(run(['ffprobe','-v','error','-show_chapters','-of','json',out]).stdout)['chapters']
            assert len(chapters)==1 and float(chapters[0]['start_time'])==0 and float(chapters[0]['end_time'])==1 and chapters[0]['tags']['title']=='First'
            record(f'{mode} preserves chapters')
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
            ('reject absent encoder',['transcode',source]),
            ('reject unavailable encoder',['transcode',source,'--encoder','fvid-no-such-encoder']),
            ('reject unused encoder option',['transcode',source,'--encoder','libx264','--encoder-option','fvid-typo=1']),
            ('reject encoder override on lossless command',['transcode-lossless',source,'--encoder','libx264']),
            ('reject video in trim-pcm',['trim-pcm',av,'--from','0','--to','1']),
            ('reject compressed trim-pcm',['trim-pcm',aac,'--streams','1','--from','0','--to','1']),
            ('reject PCM seek until audio preroll is qualified',['transcode-lossless',av,'--from','0.125','--to','0.525','--seek']),
            ('reject seek without interval',['transcode-lossless',bframes,'--seek']),
            ('reject compressed audio in lossless interval',['transcode-lossless',aac,'--from','0','--to','1']),
            ('reject empty lossless interval',['transcode-lossless',bframes,'--from','3','--to','4']),
            ('reject reversed lossless interval',['transcode-lossless',bframes,'--from','1','--to','0']),
            ('reject incomplete lossless interval',['transcode-lossless',bframes,'--from','1']),
            ('reject unqualified chapter trim',['trim',chaptered,'--from','0','--to','1']),
            ('reject non-IDR boundary',['trim',source,'--from','0.04','--to','1']),
            ('reject packet split',['trim',source,'--from','0.01','--to','1']),
            ('reject B-frame stream copy',['trim',bframes,'--from','0','--to','1']),
            ('reject incompatible concat',['concat',source,av]),
            ('reject invalid crop',['crop-lossless',source,'--crop','127:0:64:48']),
            ('reject nonexistent stream',['remux',source,'--streams','9']),
        ]:
            target=w/'rejected.mkv'
            command=[BINARY,'media',args[0],target,*args[1:]]if args[0]=='concat'else[BINARY,'media',args[0],args[1],target,*args[2:]]
            result=subprocess.run([str(v)for v in command],capture_output=True,timeout=60)
            assert result.returncode and not target.exists() and not list(w.glob('.fvid-media-*.tmp')), (name,result.stderr)
            record(name,error=result.stderr.decode())
        result=subprocess.run([str(BINARY),'media','remux',str(source),str(copied)],capture_output=True)
        assert result.returncode and hashes(source)==hashes(copied);record('existing output preserved')
    capabilities=native('capabilities')
    report=dict(created_utc=datetime.datetime.now(datetime.timezone.utc).isoformat(),status='passed',checks=checks,
                capability_counts={k:len(v)for k,v in capabilities.items()if isinstance(v,list)},
                capability_note='Inventory of linked library; not proof of implemented or tested Fvid workflows.',
                library_version=capabilities['library_version'],binary_sha256=hashlib.sha256(BINARY.read_bytes()).hexdigest(),
                source_sha256={str(p.relative_to(ROOT)):hashlib.sha256(p.read_bytes()).hexdigest() for p in [*ROOT.joinpath('src').rglob('*.rs'),*ROOT.joinpath('crates/fvid-media/src').rglob('*.rs'),ROOT/'crates/fvid-media/build.rs',ROOT/'Cargo.toml',ROOT/'Cargo.lock',ROOT/'crates/fvid-media/Cargo.toml',pathlib.Path(__file__)]})
    (ROOT/'benchmarks/media-validation.json').write_text(json.dumps(report,indent=2)+'\n')
    print(f'{len(checks)} checks passed',flush=True)
if __name__=='__main__':main()
