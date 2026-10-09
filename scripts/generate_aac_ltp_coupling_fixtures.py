#!/usr/bin/env python3
"""Own AOT4 CCE syntax videos; no private media, FFmpeg or foreign decoder."""
import json
from generate_aac_main_tools_fixtures import channel,ics,sc
from generate_he_aac_packet_fixtures import DEST,field,frequency,packed,video_fixture

def program(channels,point):
 prefix=field(4,5)+frequency(24000)+'0000'+'000'
 data=prefix+field(0,4)+field(3,2)+frequency(24000)+field(1,4)+field(0,4)+field(0,4)+field(0,2)+field(0,3)+field(1,4)+'000'+field(channels==2,1)+field(0,4)+field(point==3,1)+field(1,4)
 return packed(data+'0'*(-len(data)%8)+field(0,8))

def main():
 blob=bytearray();cases=[]
 variants=[(1,2,p,False) for p in (0,1,3)]+[(2,s,p,False) for p in (0,1,3) for s in range(4)]+[(2,3,p,True) for p in (0,1)]
 for channels,selection,point,signed in variants:
  name=f'{channels}-{point}-{selection}-{int(signed)}';rows=[]
  for frame in range(12):
   shape=frame%2;active=frame>=3 and frame%3!=1
   info=ics(0,2,False,shape=shape)
   source_info=info[:-1]+'11'+field(1024-frame,11)+field(frame%8,3)+'1'+field(frame%2==0,1) if active else info
   q=[(-1 if (frame+k)%3==0 else 1 if (frame+k)%3==1 else 0) for k in range(8)]
   body=channel(0,[1,1],[q],info=source_info)
   target=('0000000'+channel(0,[1,1],[[1,-1,1,-1,0,1,-1,1]],info=info)) if channels==1 else ('0010000'+'1'+info+'00'+channel(0,[1,1],[[1,-1,1,-1,0,1,-1,1]],info='')+channel(0,[1,1],[[-1,0,1,1,-1,1,0,-1]],info=''))
   cce='010'+field(1,4)+field(point==3,1)+'000'+field(channels==2,1)+field(0,4)+(field(selection,2) if channels==2 else '')+field(point==1,1)+field(signed,1)+'10'+body
   if channels==2 and selection==3:
    cce+=('0'+sc(3)+sc(2)) if signed else (('' if point==3 else '1')+sc(2))
   cce_start=0 if frame%2 else len(target)
   raw=packed((cce+target if frame%2 else target+cce)+'111')
   rows.append(dict(offset=len(blob),bytes=len(raw),cce_start=cce_start+3,active=active,shape=shape,lag=1024-frame,coefficient=frame%8,used=[True,frame%2==0],quantized=q));blob.extend(raw)
  case=dict(name=name,channels=channels,point=point,selection=selection,signed=signed,asc=program(channels,point).hex(),frames=rows,container_rate=24000,container_frame_samples=1024,slots=16,bands=32,samples=12288,pcm_offset=0)
  case['video']=video_fixture([case],blob,channels=channels,filename=f'aac-ltp-coupling-{name}-synthetic.mp4');cases.append(case)
 (DEST/'aac-ltp-coupling-packets.bin').write_bytes(blob)
 (DEST/'aac-ltp-coupling.json').write_text(json.dumps(dict(cases=cases,provenance='Own ordinary LTP CCE syntax, all points and stereo selections, alternating wire order, independent prediction flags/lag/gain/usage, common and signed band gain lists; packet parsing acceptance only, native coupling playback remains pending.'),indent=2)+'\n')
 print('generated',len(cases),'own LTP CCE videos')
if __name__=='__main__':main()
