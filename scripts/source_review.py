#!/usr/bin/env python3
import concurrent.futures,json,pathlib,subprocess
ROOT=pathlib.Path(__file__).resolve().parents[1]
NAMES=['FFmpeg/FFmpeg','GStreamer/gstreamer','vapoursynth/vapoursynth','pdeljanov/Symphonia','MetalPetal/MetalPetal','BabitMF/bmf','software-mansion/smelter','chromeos/cros-codecs','Vanilagy/mediabunny','membraneframework/membrane_core','BillyDM/Firewheel','rust-av/Av1an','cool-japan/oximedia','rust-av/rust-av','mozilla/mp4parse-rust','Halide/Halide','libvips/libvips','AcademySoftwareFoundation/OpenTimelineIO']
def api(endpoint):
 p=subprocess.run(['gh','api',endpoint],capture_output=True,text=True,check=True,timeout=60);return json.loads(p.stdout)
def tree(name):
 d=ROOT/'research/sources'/name.replace('/','__');d.mkdir(exist_ok=True)
 path=d/'tree.json'
 if path.exists(): return
 commit=api('repos/'+name+'/commits?per_page=1')[0]['sha']
 data=api('repos/'+name+'/git/trees/'+commit+'?recursive=1')
 data['pinned_commit']=commit;path.write_text(json.dumps(data,indent=2))
 print(name,commit[:12],len(data.get('tree',[])),data.get('truncated'),flush=True)
if __name__=='__main__':
 with concurrent.futures.ThreadPoolExecutor(max_workers=5) as pool:
  for _ in pool.map(tree,NAMES): pass
