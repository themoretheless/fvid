#!/usr/bin/env python3
"""Read-only GitHub collection. Never runs code or instructions from repositories."""
import base64,concurrent.futures,datetime,hashlib,json,pathlib,subprocess,time
ROOT=pathlib.Path(__file__).resolve().parents[1]
EXTRA=['VideoLAN/vlc','HandBrake/HandBrake','libav/libav','mpv-player/mpv','mltframework/mlt','OpenShot/libopenshot','obsproject/obs-studio','Netflix/vmaf','xiph/rav1e','xiph/opus','xiph/flac','xiph/ogg','xiph/vorbis','xiph/daala','videolan/dav1d','videolan/x264','videolan/libplacebo','ShiftMediaProject/x265','AOMediaCodec/SVT-AV1','cisco/openh264','webmproject/libvpx','ultravideo/kvazaar','fraunhoferhhi/vvenc','fraunhoferhhi/vvdec','FFmpeg/nv-codec-headers','rust-av/rav1e','rust-av/v_frame','rust-av/av-data','rust-av/av-format','rust-av/y4m','rust-av/av-codec','rust-av/rust-av','rust-av/av-io','rust-av/av-metrics','Vanilagy/mediabunny','ashellunts/ffmpeg-to-webrtc','image-rs/image','image-rs/imageproc','AWxLab/fast_image_resize','awxkee/yuvutils-rs','awxkee/pic-scale','libjxl/libjxl','AcademySoftwareFoundation/OpenTimelineIO','AcademySoftwareFoundation/OpenColorIO','AcademySoftwareFoundation/OpenEXR','AcademySoftwareFoundation/OpenImageIO','Halide/Halide','libvips/libvips','PixarAnimationStudios/OpenTimelineIO','PipeWire/pipewire','membraneframework/membrane_core','membraneframework/membrane_rtc_engine','membraneframework/membrane_transcoder_plugin','GStreamer/gst-plugins-rs','GStreamer/gstreamer-rs','RReverser/libwebp-rs','lieff/miniaudio','mackron/miniaudio','RustAudio/rubato','orottier/web-audio-api-rs','Timmmm/ez-audio','rust-media/gst-plugin','mstorsjo/fdk-aac','knik0/faad2','libsndfile/libsndfile','breakfastquay/rubberband','alesgenova/rust_poly','icedland/iced','software-mansion/smelter','cool-japan/oximedia','pdeljanov/Symphonia','BabitMF/bmf','twitter/vireo','ScuffleCloud/scuffle','chromeos/cros-codecs']
def api(endpoint):
 p=subprocess.run(['gh','api',endpoint],capture_output=True,text=True,timeout=45)
 if p.returncode: raise RuntimeError(p.stderr.strip()[:250])
 return json.loads(p.stdout)
def metadata(repo):
 path=ROOT/'research/raw'/('extra-'+repo.replace('/','__')+'.json')
 try:
  if path.exists(): return json.load(open(path))
  d=api('repos/'+repo);path.write_text(json.dumps(d,ensure_ascii=False,indent=2));return d
 except Exception as e: return {'requested':repo,'error':str(e)}
def readme(repo):
 path=ROOT/'research/sources'/repo.replace('/','__');path.mkdir(exist_ok=True)
 provenance=path/'provenance.json'
 if provenance.exists(): return json.load(open(provenance))
 result={'repository':repo,'retrieved_at_utc':datetime.datetime.now(datetime.timezone.utc).isoformat()}
 try:
  d=api('repos/'+repo+'/readme');raw=base64.b64decode(d['content']); (path/'README.txt').write_bytes(raw)
  result.update(status='ok',blob_sha=d['sha'],path=d['path'],url=d['html_url'],sha256=hashlib.sha256(raw).hexdigest(),bytes=len(raw))
 except Exception as e: result.update(status='failed',error=str(e))
 provenance.write_text(json.dumps(result,ensure_ascii=False,indent=2));return result
if __name__=='__main__':
 catalog=json.load(open(ROOT/'research/catalog.json'))
 with concurrent.futures.ThreadPoolExecutor(max_workers=6) as pool:
  extras=list(pool.map(metadata,EXTRA))
 json.dump(extras,open(ROOT/'research/extra-index.json','w'),ensure_ascii=False,indent=2)
 names={r['full_name'] for r in catalog}
 names.update(r['full_name'] for r in extras if 'full_name' in r)
 results=[]
 with concurrent.futures.ThreadPoolExecutor(max_workers=6) as pool:
  for result in pool.map(readme,sorted(names)):
   results.append(result)
   if len(results)%50==0: print(f"README {len(results)}/{len(names)}, successful={sum(x['status']=='ok' for x in results)}",flush=True)
 json.dump(results,open(ROOT/'research/readme-index.json','w'),ensure_ascii=False,indent=2)
 print('COMPLETE',len(results),'successful',sum(x['status']=='ok' for x in results),flush=True)
