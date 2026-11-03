"""Explicit temporary benchmark corpus; ordinary regressions never generate media."""
from pathlib import Path
import argparse,json,hashlib,struct
from generate_wave_ima_playback import generate as ima,riff
from generate_wave_ms_adpcm_playback import generate as ms
if __name__=='__main__':
 p=argparse.ArgumentParser();p.add_argument('--output',required=True);a=p.parse_args();out=Path(a.output);out.mkdir(parents=True,exist_ok=True);files={};cases=[]
 for family,parent in [('ima',ima()),('ms',ms())]:
  for channels in [1,2]:
   c=next(c for c in json.loads(parent['manifest.json'])['cases']if c['channels']==channels and c['block_bytes']==8*channels and c['mode']=='full'and c['kind']=='riff'and(family=='ima'or c['table']=='standard'))
   block=c['block_bytes'];spb=c['block_frames'];stride=channels*4;raw=parent[c['encoded']][:block];gold=parent[c['oracle']][:spb*stride]
   fmt=parent[c['setup']]if family=='ms'else struct.pack('<HHIIHHHH',17,channels,8000,8000*block//spb,block,4,2,spb)
   blocks=(120*8000+spb-1)//spb;frames=blocks*spb-min(spb-1,7);window=max(1,2048//spb)*spb;assert frames%window>25
   key=f'{family}-c{channels}';files[key+'.wav']=riff(fmt,raw*blocks,frames,'trim')
   for position,target,count in [('start',0,window),('late',frames-25,25)]:
    expected=b''.join(gold[(i%spb)*stride:(i%spb+1)*stride]for i in range(target,target+count));n=f'{key}-{position}.f32le';files[n]=expected
    cases.append(dict(name=key+'-'+position,file=key+'.wav',expected=n,position=position,target_frame=target,target_ns=target*1_000_000_000//8000,frames=frames,rate=8000,channels=channels,block_bytes=block,block_frames=spb,end_after_first=position=='late'))
 for n,b in files.items():(out/n).write_bytes(b)
 manifest=dict(provenance='Temporary120-second repeated authored ADPCM blocks, analytic first/last PCM. Cold input open excluded; warm reader seek+decoder reset to first presented PCM measured. Separate generator; no FFmpeg/network.',cases=cases,sha256={n:hashlib.sha256(b).hexdigest()for n,b in files.items()});(out/'manifest.json').write_text(json.dumps(manifest,indent=2)+'\n');print('benchmark corpus',len(cases),'workloads',len(files),'files')
