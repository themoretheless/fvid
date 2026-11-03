"""Short authored WAVE ADPCM witnesses for independent-block navigation."""
from pathlib import Path
import json,struct
from generate_wave_ima_playback import generate as ima, riff
from generate_wave_ms_adpcm_playback import generate as ms
from generate_rf64_playback import build as rf64
from generate_wave64_playback import build as wave64
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/wave-block-seek'
def generate():
 files={};cases=[]
 for family,parent in [('ima',ima()),('ms',ms())]:
  m=json.loads(parent['manifest.json']);files.update({n:parent[n]for n in ['neutral-video.mp4','frame-0.yuv16','frame-1.yuv16','frame-2.yuv16']})
  for c in m['cases']:
   if c['mode'] not in ['full','trim']:continue
   name=family+'-'+c['name'];raw=parent[c['encoded']];block=c['block_bytes'];spb=c['block_frames'];blocks=len(raw)//block;index=min(3,blocks-1);target=index*spb+1
   fmt=parent[c['setup']]if family=='ms'else struct.pack('<HHIIHHHH',17,c['channels'],c['rate'],c['rate']*block//spb,block,4,2,spb)
   def build(payload):return riff(fmt,payload,c['frames'],c['mode'])if c['kind']=='riff'else rf64(fmt,payload,c['frames'])if c['kind']=='rf64'else wave64(fmt,payload,c['frames'])
   def bad(offset):
    b=bytearray(raw)
    if family=='ima':b[offset+2]=89
    else:struct.pack_into('<h',b,offset+c['channels'],0)
    return bytes(b)
   files[name+'-good.wav']=parent[c['file']];files[name+'-earlier-bad.wav']=build(bad(0));files[name+'-target-bad.wav']=build(bad(index*block));files[name+'.pcm.wav']=parent[c['control']];files[name+'.f32le']=parent[c['oracle']]
   cases.append(dict(name=name,family=family,frames=c['frames'],rate=c['rate'],channels=c['channels'],block_bytes=block,block_frames=spb,data_bytes=len(raw),target_frame=target,target_ns=(target-1)*1_000_000_000//c['rate']+1,needle='step index exceeds 88'if family=='ima'else'initial delta must be positive'))
 files['manifest.json']=(json.dumps(dict(provenance='Authored IMA/MS WAVE block signals and analytic PCM controls. Earlier and selected block headers fail independently; no private input, encoder or network.',cases=cases),indent=2)+'\n').encode();return files
if __name__=='__main__':
 OUT.mkdir(parents=True,exist_ok=True)
 for n,b in generate().items():(OUT/n).write_bytes(b)
