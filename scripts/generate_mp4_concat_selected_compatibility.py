"""Owned AVC segments with compatible video and differing discarded AAC clocks."""
from pathlib import Path
import hashlib,json
from generate_mp4_audio_selection_regressions import boxes,build_aac_pulse_packet,build_esds,make_audio_track,mp4a_v0_header
from generate_vp9_chroma import atom
root=Path(__file__).resolve().parents[1]
source=root/'tests/fixtures/playback-errors/avc-444-pcm-deblock/y8-c8-o12-12-m1-e0.mp4'
out=root/'tests/fixtures/playback-errors/mp4-concat-selected-compatibility';out.mkdir(exist_ok=True)
parts=dict(boxes(source.read_bytes()));ftyp=atom(b'ftyp',parts[b'ftyp']);mdat=atom(b'mdat',parts[b'mdat'])
base=b''.join(atom(k,v[:-4]+(3).to_bytes(4,'big') if k==b'mvhd' else v) for k,v in boxes(parts[b'moov']))
pulse=build_aac_pulse_packet(140,1,7);packets=[pulse]*2
files={}
for rate,asc in [(48000,b'\x11\x88'),(44100,b'\x12\x08')]:
 entry=mp4a_v0_header(1,rate)+atom(b'esds',build_esds(asc))
 placeholder=make_audio_track(2,3,b'mp4a',entry,packets,0,rate)
 offset=len(ftyp)+len(mdat)+8+len(base)+len(placeholder)+8
 track=make_audio_track(2,3,b'mp4a',entry,packets,offset,rate)
 data=ftyp+mdat+atom(b'moov',base+track)+atom(b'mdat',b''.join(packets))
 name=f'video-audio-{rate}.mp4';(out/name).write_bytes(data);files[name]=hashlib.sha256(data).hexdigest()
data=ftyp+mdat+atom(b'moov',base)
(out/'video-only.mp4').write_bytes(data);files['video-only.mp4']=hashlib.sha256(data).hexdigest()
(out/'manifest.json').write_text(json.dumps({'sha256':files,'video_source':str(source.relative_to(root)),'provenance':'Own PCM-coded AVC and AAC-LC pulse syntax. Video configuration and bytes identical. AAC sample entry and AudioSpecificConfig correctly state 48kHz vs 44.1kHz.'},indent=2)+'\n')
