#!/usr/bin/env python3
"""Original PS video wrappers with unequal packet presentation durations.
Explicit offline generation, own authored packets and video/container seeds.
"""
import json
from generate_he_aac_packet_fixtures import DEST,video_fixture

def main():
 sources=[json.loads((DEST/'aac-ps-matrix-controller-oracles.json').read_text())['videos'][0],json.loads((DEST/'aac-sbr-ps-30-oracles.json').read_text())['videos'][0]]
 cases=[]
 for v in sources:
  slots=v.get('slots',16);full=slots*128;durations=[full*3//4,full//2,full]
  packet_file=v.get('packet_file','he-aac-ps-matrix-controller-packets.bin');blob=(DEST/packet_file).read_bytes()
  c=dict(slots=slots,bands=64,frames=v['packet_frames'],asc=v['video']['asc'],pcm_offset=0,samples=full*3*2,durations=durations)
  video=video_fixture([c],blob,channels=2,filename=f'he-aac-ps-timing-{slots*64}-synthetic.mp4');video.pop('pcm_offset');video.pop('samples')
  cases.append(dict(video=video,source_reference=v['video']['file'],packet_file=packet_file,frames=v['packet_frames'],slots=slots,durations=durations,pts=[0,durations[0],sum(durations[:2])]))
 (DEST/'aac-ps-playback-oracles.json').write_text(json.dumps(dict(kind='original unequal presentation windows for delayed native PS frames',cases=cases),indent=2)+'\n')
 print('Two original unequal-duration PS playback videos')
if __name__=='__main__':main()
