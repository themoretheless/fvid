#!/usr/bin/env python3
"""Explicit external-reference benchmark diagnostic; never an ordinary test.
No reference/fixture generation, no runtime library dependency on FFmpeg.
"""
import argparse,hashlib,json,os,struct,subprocess,shutil,re,platform
from pathlib import Path
ROOT=Path(__file__).resolve().parents[3]
def digest(b):return hashlib.sha256(b).hexdigest()
def rgb(data,depth):
 values=list(data)if depth==8 else list(struct.unpack('<'+'H'*(len(data)//2),data))
 return [v for i,v in enumerate(values)if i%4!=3]
def main():
 p=argparse.ArgumentParser();p.add_argument('--output',type=Path,required=True);a=p.parse_args()
 exe=os.environ.get('FVID_FFMPEG','ffmpeg');version=subprocess.run([exe,'-version'],capture_output=True,check=True,text=True).stdout
 rows=[]
 for depth in [8,16]:
  source=ROOT/'tests/fixtures/playback-errors'/f'vibrance-grid-{depth}.rgba';data=source.read_bytes();fmt='rgba'if depth==8 else'rgba64le';original=rgb(data,depth);outputs={};graphs={}
  filters={'null':'null','exposure_identity':'exposure=0','float_rgb_roundtrip':'format=gbrpf32le','float_rgba_roundtrip':'format=gbrapf32le','exposure_half':'exposure=-1'}
  for name,graph in filters.items():
   cmd=[exe,'-nostdin','-v','verbose','-f','rawvideo','-pixel_format',fmt,'-video_size','8x8','-i','pipe:0','-vf',graph,'-frames:v','3','-pix_fmt',fmt,'-f','rawvideo','pipe:1']
   r=subprocess.run(cmd,input=data,capture_output=True,check=True);assert len(r.stdout)==len(data)
   outputs[name]=r.stdout;graphs[name]=[re.sub(r'@ 0x[0-9a-fA-F]+', '@ <instance>', s) for s in r.stderr.decode().splitlines()if any(k in s for k in ['auto_scale','fmt:','pixfmt:'])]
  assert outputs['null']==data,'raw reference control changes input'
  identity=rgb(outputs['exposure_identity'],depth);matches=[k for k in filters if k.startswith('float_')and rgb(outputs[k],depth)==identity]
  assert matches,'identity exposure differs from both explicit conversion controls'
  stored=(ROOT/'tests/fixtures/playback-errors'/f'exposure-reference-rgb-{depth}-0.raw').read_bytes();current_rgb=bytes(identity)if depth==8 else struct.pack('<'+'H'*len(identity),*identity)
  base=[exe,'-nostdin','-v','error','-f','rawvideo','-pixel_format',fmt,'-video_size','8x8','-i','pipe:0','-vf','format=gbrapf32le','-frames:v','3','-pix_fmt','gbrapf32le','-f','rawvideo','pipe:1']
  floats=subprocess.run(base,input=data,capture_output=True,check=True).stdout
  assert len(floats)==3*256*4
  values=list(struct.unpack('<768f',floats))
  for frame in range(3):
   for i in range(frame*256,frame*256+192):values[i]*=0.5
  half=struct.pack('<768f',*values)
  back=[exe,'-nostdin','-v','error','-f','rawvideo','-pixel_format','gbrapf32le','-video_size','8x8','-i','pipe:0','-frames:v','3','-pix_fmt',fmt,'-f','rawvideo','pipe:1']
  half_roundtrip=subprocess.run(back,input=half,capture_output=True,check=True).stdout
  assert rgb(half_roundtrip,depth)==rgb(outputs['exposure_half'],depth),'isolated conversion/half-kernel pipeline differs'
  row=dict(depth=depth,half_pipeline_rgb_exact=True,converted_float_sha256=digest(floats),half_float_sha256=digest(half),half_roundtrip_sha256=digest(half_roundtrip),input_sha256=digest(data),input_samples=len(original),identity_changed_samples=sum(x!=y for x,y in zip(original,identity)),identity_max_delta=max(abs(x-y)for x,y in zip(original,identity)),matching_float_roundtrips=matches,stored_identity_rgb_matches_current=stored==current_rgb,equal_code_value_variants_across_channels=sum(len({y for x,y in zip(original,identity)if x==v})>1 for v in set(original)),outputs_sha256={k:digest(v)for k,v in outputs.items()},conversion_graphs=graphs)
  maximum=(1<<depth)-1;constant=maximum//2
  authored=[v for i in range(192)for v in [constant,(i*71+17)% (maximum+1),(i*131+31)% (maximum+1),maximum]]
  authored=bytes(authored)if depth==8 else struct.pack('<768H',*authored)
  isolated_float=subprocess.run(base,input=authored,capture_output=True,check=True).stdout
  isolated_out=subprocess.run(back,input=isolated_float,capture_output=True,check=True).stdout
  red=rgb(isolated_out,depth)[::3]
  row['constant_red_control']=dict(input_red=constant,output_red_min=min(red),output_red_max=max(red),distinct_output_red=len(set(red)),input_sha256=digest(authored),float_sha256=digest(isolated_float),output_sha256=digest(isolated_out),scope='Own192-pixel three-frame arithmetic input; red constant, green/blue varied. Isolates conversion context sensitivity without the exposure kernel.')
  rows.append(row);print(depth,row['identity_changed_samples'],row['identity_max_delta'],matches,'constant red range',min(red),max(red),flush=True)
 report=dict(platform=platform.platform(),machine=platform.machine(),scope='External RGB conversion isolation, not native integer-conversion acceptance or a decoder fix',ffmpeg=version,executable=str(Path(shutil.which(exe)).resolve()),executable_sha256=digest(Path(shutil.which(exe)).read_bytes()),cases=rows,diagnostic_sha256=digest(Path(__file__).read_bytes()),fixtures_written=False)
 a.output.write_text(json.dumps(report,indent=2)+'\n')
if __name__=='__main__':main()
