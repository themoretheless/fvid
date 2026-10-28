"""Owned 40 ms AVC sample with a large, legal moov/free metadata padding box."""
from pathlib import Path
import hashlib,json
from generate_mp4_audio_selection_regressions import boxes
from generate_vp9_chroma import atom
root=Path(__file__).resolve().parents[1]
source=root/'tests/fixtures/playback-errors/avc-444-pcm-deblock/y8-c8-o12-12-m1-e0.mp4'
out=root/'tests/fixtures/playback-errors/mp4-metadata-admission';out.mkdir(exist_ok=True)
base=source.read_bytes();files={'small.mp4':base}
files['padded.mp4']=b''.join(atom(k,v+atom(b'free',bytes(1<<20)) if k==b'moov' else v) for k,v in boxes(base))
unknown=bytearray(base);at=base.index(b'stsd');unknown[at+16:at+20]=b'fvdx'
files['unsupported-entry.mp4']=bytes(unknown)
for name,data in files.items():(out/name).write_bytes(data)
(out/'manifest.json').write_text(json.dumps({'sha256':{name:hashlib.sha256(data).hexdigest() for name,data in files.items()},'source':str(source.relative_to(root)),'provenance':'Owned 32x32 PCM-coded AVC. Add one legal 1MiB moov/free box; original media, track tables and mdat offsets unchanged. No private media, FFmpeg or network.'},indent=2)+'\n')
