"""Author delayed AVI ADPCM streams; no encoder, private media or network."""
from pathlib import Path
import json,struct
from generate_avi_adpcm_grouped_regressions import generate as grouped
from generate_mp4_seek_io_regressions import generate as video
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/avi-adpcm-start'
def generate():
 own=grouped();v=video();files={'neutral-video.mp4':v['three-colors.mp4'],**{f'frame-{i}.yuv16':v[f'frame-{i}.yuv16']for i in range(3)}};m=json.loads(own['manifest.json']);cases=[]
 for c in m['controls']+m['cases']:
  base=own[c['file']];name=c['file'].removesuffix('.avi');header=base.index(b'auds');files[c['file']]=base;files[c['expected']]=own[c['expected']]
  for mode in ['original','equivalent']:
   for delay in [0,1,73,0xffffffff]:
    data=bytearray(base);scale,rate=struct.unpack_from('<II',data,header+20)
    if mode=='equivalent':
     if scale%4==0 and rate%4==0:scale//=4;rate//=4
     else:scale*=2;rate*=2
     struct.pack_into('<II',data,header+20,scale,rate)
    struct.pack_into('<I',data,header+28,delay);file=f'{name}-start{delay}-{mode}.avi';files[file]=bytes(data)
    case=dict(c);case.update(file=file,control=c['file'],origin=delay*c['block_frames'],start=delay,clock_mode=mode,frames=c['frames'],rate=8000,block_frames=c['block_frames']);cases.append(case)
 bad=[]
 for c in m['malformed']:
  data=bytearray(own[c['file']]);header=data.index(b'auds');struct.pack_into('<I',data,header+28,73);file='delay-'+c['file'];files[file]=bytes(data);bad.append(dict(file=file,needle='whole ADPCM blocks' if c['reason']=='incomplete' else 'ADPCM blocks and the run holds'))
 files['manifest.json']=(json.dumps(dict(provenance='Authored VP9+IMA/MS AVI, mono/stereo, grouped/single/nested/indexed records; dwStart0/1/73/u32MAX in original/equivalent dwScale/dwRate units, literal PCM controls. No encoder, private media or network.',cases=cases,bad=bad),indent=2)+'\n').encode();return files
if __name__=='__main__':
 OUT.mkdir(parents=True,exist_ok=True)
 for n,b in generate().items():(OUT/n).write_bytes(b)
