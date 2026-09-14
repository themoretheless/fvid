#!/usr/bin/env python3
import base64,concurrent.futures,json,pathlib,subprocess,hashlib
ROOT=pathlib.Path(__file__).resolve().parents[1]
FILES={
'FFmpeg/FFmpeg':['fftools/ffmpeg_sched.h','libavfilter/vf_crop.c','libavfilter/vf_vflip.c','libavutil/frame.h'],
'GStreamer/gstreamer':['subprojects/gstreamer/gst/gstbufferpool.c','subprojects/gstreamer/plugins/elements/gstqueue.c'],
'vapoursynth/vapoursynth':['include/VapourSynth4.h'],
'pdeljanov/Symphonia':['symphonia-core/src/io/media_source_stream.rs','symphonia-core/src/units.rs'],
'MetalPetal/MetalPetal':['Frameworks/MetalPetal/MTIRenderGraphOptimization.m','Frameworks/MetalPetal/MTIImagePromise.h'],
'BabitMF/bmf':['bmf/engine/c_engine/include/scheduler_queue.h','bmf/sdk/cpp_sdk/include/bmf/sdk/video_frame.h'],
'software-mansion/smelter':['gpu-video/src/decoders/wgpu_api.rs','gpu-video/src/frame_sorter.rs'],
'chromeos/cros-codecs':['src/decoder.rs','src/backend/vaapi/surface_pool.rs'],
'Vanilagy/mediabunny':['src/conversion.ts','src/media-source.ts'],
'membraneframework/membrane_core':['lib/membrane/core/element/manual_flow_controller.ex','lib/membrane/core/element/manual_flow_controller/input_queue.ex'],
'BillyDM/Firewheel':['crates/firewheel-graph/src/graph/compiler/schedule.rs','crates/firewheel-graph/src/processor/process.rs'],
'rust-av/Av1an':['av1an-core/src/broker.rs','av1an-core/src/chunk/mod.rs'],
'cool-japan/oximedia':['docs/codec_status.md','crates/oximedia-graph/src/filters/video/crop.rs'],
'rust-av/rust-av':['data/src/frame.rs','codec/src/lib.rs'],
'mozilla/mp4parse-rust':['mp4parse/src/lib.rs'],
'halide/Halide':['tutorial/lesson_08_scheduling_2.cpp'],
'libvips/libvips':['libvips/iofuncs/generate.c'],
'AcademySoftwareFoundation/OpenTimelineIO':['src/opentime/rationalTime.h']}
def fetch(task):
 name,file=task;directory=ROOT/'research/sources'/name.replace('/','__');tree=json.load(open(directory/'tree.json'));commit=tree['pinned_commit']
 p=subprocess.run(['gh','api','repos/'+name+'/contents/'+file+'?ref='+commit],capture_output=True,text=True,timeout=60)
 if p.returncode:return {'repository':name,'file':file,'error':p.stderr[:200]}
 d=json.loads(p.stdout);content=base64.b64decode(d['content']);dest=directory/'code'/file;dest.parent.mkdir(parents=True,exist_ok=True);dest.write_bytes(content)
 return {'repository':name,'file':file,'commit':commit,'blob_sha':d['sha'],'sha256':hashlib.sha256(content).hexdigest(),'url':f'https://github.com/{name}/blob/{commit}/{file}','local':str(dest.relative_to(ROOT)),'lines':len(content.splitlines())}
with concurrent.futures.ThreadPoolExecutor(max_workers=5) as pool: results=list(pool.map(fetch,[(n,p) for n,ps in FILES.items() for p in ps]))
json.dump(results,open(ROOT/'research/source-index.json','w'),indent=2)
for r in results: print(r['repository'],r['file'],r.get('lines',r.get('error')))
