"""Owned mono PCM MP4 with legal metadata padding; no external tools."""
from pathlib import Path
import hashlib,json
from generate_mp4_audio_max_packets_budget import build_mp4_sowt_prefix_m4a
from generate_mp4_audio_selection_regressions import boxes
from generate_vp9_chroma import atom
root=Path(__file__).resolve().parents[1]
out=root/'tests/fixtures/playback-errors/mp4-audio-metadata-admission';out.mkdir(exist_ok=True)
base=build_mp4_sowt_prefix_m4a()
files={'small.m4a':base,'padded.m4a':b''.join(atom(k,v+atom(b'free',bytes(2<<20)) if k==b'moov' else v) for k,v in boxes(base))}
for name,data in files.items():(out/name).write_bytes(data)
(out/'manifest.json').write_text(json.dumps({'sha256':{name:hashlib.sha256(data).hexdigest() for name,data in files.items()},'provenance':'Owned 136 signed PCM samples at 8kHz mono, 17ms duration. Legal 2MiB moov/free padding does not move any encoded media. No private audio, video, codec parameters, FFmpeg or network.'},indent=2)+'\n')
