"""Owned AAC edit retaining two packets from a 512-packet track; offline."""
from pathlib import Path
import hashlib, json
from generate_mp4_audio_selection_regressions import build_aac_pulse_packet
from generate_mp4_matroska_encoder_concat_prefix import build_mp4_aac_segment
from generate_mp4_variable_rate_edits import edit_movie
root=Path(__file__).resolve().parents[1]
out=root/'tests/fixtures/playback-errors/mp4-remux-plan-capacity'
out.mkdir(exist_ok=True)
pulse=build_aac_pulse_packet(140,1,7)
data=edit_movie(build_mp4_aac_segment([pulse]*512,'owned edit','fvid synthetic'),[(2048,0)],clock=48000)
(out/'edited.m4a').write_bytes(data)
(out/'manifest.json').write_text(json.dumps({'file':'edited.m4a','sha256':hashlib.sha256(data).hexdigest(),'provenance':'Owned 48 kHz mono AAC pulse syntax; 512 packets with one 2048-sample edit. No private media, FFmpeg or network.'},indent=2)+'\n')
