"""Owned VP9 fixture with many harmless top-level free boxes; no FFmpeg."""
from pathlib import Path
import hashlib, json
root = Path(__file__).resolve().parents[1]
out = root / "tests/fixtures/playback-errors/mp4-relocation-admission"
out.mkdir(exist_ok=True)
source = root / "tests/fixtures/playback-errors/mp4-signed-offset/positive.mp4"
data = source.read_bytes() + b"\0\0\0\x08free" * 10000
(out / "many-boxes.mp4").write_bytes(data)
(out / "manifest.json").write_text(json.dumps({"file": "many-boxes.mp4", "sha256": hashlib.sha256(data).hexdigest(), "source": str(source.relative_to(root)), "construction": "Append 10000 empty free boxes without moving encoded samples or changing fragment offsets"}, indent=2) + "\n")
