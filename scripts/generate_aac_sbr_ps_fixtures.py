#!/usr/bin/env python3
"""Original 960-sample SCE/SBR/PS MP4s and independent 30-slot parameters.
Offline explicit generation, no private media or foreign codec implementation.
"""
import json,hashlib
from generate_aac_ps_matrix_controller_fixtures import Store,timeline,phase_trace,mapped,choose,targets,zero,encode
from generate_aac_ps_fixtures import sbr
from generate_he_aac_packet_fixtures import DEST,packet,asc,video_fixture

def main():
 store=Store();blob=bytearray();videos=[]
 phase_grid=[r['value'] for r in json.loads((DEST/'aac-ps-dequant-oracles.json').read_text())['phase']]
 for name,controls in [('retain',[(1,2,True),(1,0,False),(1,0,True)]),('grid-retain',[(1,2,True),(5,0,True),(1,0,False)])]:
  before=zero();previous=20;frames=[];payloads=[];packets=[]
  for stage,(mode,count,enabled) in enumerate(controls):
   rows=[targets(mode,mode,stage*7+e,phase=enabled) for e in range(count)]
   text,borders=encode(True,mode,mode,True,True,enabled,rows,before,30)
   raw=sbr(text,stage);data=packet(raw);packets.append(dict(offset=len(blob),bytes=len(data)));blob.extend(data);payloads.append(raw.hex());bands=choose(previous,mode,mode)
   frames.append(dict(previous_bands=previous,bands=bands,initialized=True,phase_enabled=enabled,borders=borders,envelopes=[mapped(v,bands,phase_grid) for v in rows]));previous=bands
   if rows:before=rows[-1]
  case=dict(slots=15,bands=64,frames=packets,asc=asc(24000,48000,15,'explicit',ps=True).hex(),pcm_offset=0,samples=11520)
  video=video_fixture([case],blob,channels=2,filename=f'he-aac-sbr-ps-960-{name}-synthetic.mp4');video.pop('pcm_offset');video.pop('samples')
  expected=timeline(frames,phase_trace(frames),30,store)
  for f in expected:f['coefficients_file']='aac-sbr-ps-30-coefficients.bin'
  videos.append(dict(video=video,slots=15,packet_file='he-aac-sbr-ps-960-packets.bin',sbr_payloads=payloads,packet_frames=packets,expected=expected))
 (DEST/'he-aac-sbr-ps-960-packets.bin').write_bytes(blob)
 (DEST/'aac-sbr-ps-30-coefficients.bin').write_bytes(store.data)
 (DEST/'aac-sbr-ps-30-oracles.json').write_text(json.dumps(dict(kind='original 960-sample silent SCE and authored SBR/PS packets; independent matrix coefficients',videos=videos,packet_sha256=hashlib.sha256(blob).hexdigest(),coefficient_sha256=hashlib.sha256(store.data).hexdigest()),indent=2)+'\n')
 print('Two original 960-sample PS MP4s and 30-slot coefficient references')
if __name__=='__main__':main()
