"""Owned AVC/AAC with a conservative empty video sync-sample table."""
from pathlib import Path
import hashlib,json
from generate_mp4_audio_selection_regressions import boxes,build_aac_pulse_packet,build_esds,make_audio_track,mp4a_v0_header
from generate_vp9_chroma import atom
root=Path(__file__).resolve().parents[1]
source=root/'tests/fixtures/playback-errors/avc-444-pcm-deblock/y8-c8-o12-12-m1-e0.mp4'
out=root/'tests/fixtures/playback-errors/mp4-concat-discarded-sync';out.mkdir(exist_ok=True)
parts=dict(boxes(source.read_bytes()))
def rewrite(data):
 result=b''
 for kind,body in boxes(data):
  if kind in [b'trak',b'mdia',b'minf',b'stbl']:body=rewrite(body)
  elif kind==b'stss':body=bytes(8)
  elif kind==b'mvhd':body=body[:-4]+(3).to_bytes(4,'big')
  result+=atom(kind,body)
 return result
base=rewrite(parts[b'moov']);ftyp=atom(b'ftyp',parts[b'ftyp']);mdat=atom(b'mdat',parts[b'mdat'])
pulse=build_aac_pulse_packet(140,1,7);packets=[pulse]*2
entry=mp4a_v0_header(1,48000)+atom(b'esds',build_esds(b'\x11\x88'))
placeholder=make_audio_track(2,3,b'mp4a',entry,packets,0)
offset=len(ftyp)+len(mdat)+8+len(base)+len(placeholder)+8
track=make_audio_track(2,3,b'mp4a',entry,packets,offset)
data=ftyp+mdat+atom(b'moov',base+track)+atom(b'mdat',b''.join(packets))
(out/'unmarked-video-sync.mp4').write_bytes(data)
(out/'manifest.json').write_text(json.dumps({'file':'unmarked-video-sync.mp4','sha256':hashlib.sha256(data).hexdigest(),'provenance':'Owned PCM AVC bytes with empty stss (no declared video sync point), plus own 48kHz mono AAC pulse syntax. Only selected audio needs concatenation. Video selection must preserve existing sync-point refusal.'},indent=2)+'\n')
