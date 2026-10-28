"""Owned 40ms AVC picture plus an unselected 10.923s AAC pulse track."""
from pathlib import Path
import hashlib,json
from generate_mp4_audio_selection_regressions import boxes,build_aac_pulse_packet,build_esds,make_audio_track,mp4a_v0_header
from generate_vp9_chroma import atom
root=Path(__file__).resolve().parents[1]
source=root/'tests/fixtures/playback-errors/avc-444-pcm-deblock/y8-c8-o12-12-m1-e0.mp4'
out=root/'tests/fixtures/playback-errors/mp4-concat-selected-clock';out.mkdir(exist_ok=True)
parts=dict(boxes(source.read_bytes()));ftyp=atom(b'ftyp',parts[b'ftyp']);mdat=atom(b'mdat',parts[b'mdat'])
pulse=build_aac_pulse_packet(140,1,7);packets=[pulse]*512
entry=mp4a_v0_header(1,48000)+atom(b'esds',build_esds(b'\x11\x88'))
base=b''.join(atom(k,v[:-4]+(3).to_bytes(4,'big') if k==b'mvhd' else v) for k,v in boxes(parts[b'moov']))
placeholder=make_audio_track(2,3,b'mp4a',entry,packets,0)
offset=len(ftyp)+len(mdat)+8+len(base)+len(placeholder)+8
track=make_audio_track(2,3,b'mp4a',entry,packets,offset)
moov=atom(b'moov',base+track)
data=ftyp+mdat+moov+atom(b'mdat',b''.join(packets))
(out/'video-and-long-audio.mp4').write_bytes(data)
(out/'manifest.json').write_text(json.dumps({'file':'video-and-long-audio.mp4','sha256':hashlib.sha256(data).hexdigest(),'video_source':str(source.relative_to(root)),'provenance':'Owned PCM AVC fixture and own AAC pulse syntax; original video byte offsets preserved. Selecting only the short video should concatenate with 40ms cadence.'},indent=2)+'\n')
