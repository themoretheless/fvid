"""Owned two-track AAC MP4 for retained concat-plan budget admission."""
from pathlib import Path
import hashlib, json
from generate_mp4_matroska_encoder_concat_prefix import build_mp4_aac_segment
from generate_mp4_audio_selection_regressions import build_aac_pulse_packet, build_esds, make_audio_track, mp4a_v0_header
from generate_vp9_chroma import atom, word
root = Path(__file__).resolve().parents[1]
out = root / 'tests/fixtures/playback-errors/mp4-concat-retained-plans'
out.mkdir(exist_ok=True)
pulse = build_aac_pulse_packet(140, 1, 7)
base = build_mp4_aac_segment([pulse]*512, 'owned pulse', 'fvid synthetic')
pos = base.index(b'mvhd') - 4
mvhd = base[pos:pos+int.from_bytes(base[pos:pos+4], 'big')]
ftyp = atom(b'ftyp', b'isom'+word(0x200)+b'isommp42')
entry = mp4a_v0_header(1,48000)+atom(b'esds',build_esds(b'\x11\x88'))
offset = len(ftyp)+8
tracks = make_audio_track(1,3,b'mp4a',entry,[pulse]*2,offset)
tracks += make_audio_track(2,3,b'mp4a',entry,[pulse]*512,offset+len(pulse)*2)
data = ftyp+atom(b'mdat',pulse*514)+atom(b'moov',mvhd+tracks)
(out/'two-tracks.m4a').write_bytes(data)
(out/'manifest.json').write_text(json.dumps({'file':'two-tracks.m4a','sha256':hashlib.sha256(data).hexdigest(),'provenance':'Owned synthetic mono 48kHz AAC pulse, 2 and 512 samples on separate tracks; no FFmpeg or private media.'},indent=2)+'\n')
