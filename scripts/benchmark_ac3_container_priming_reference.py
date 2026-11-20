"""Explicit FFmpeg-only authoring/reference generation for AC-3 container priming."""
import hashlib
import json
import subprocess
import tempfile
from pathlib import Path

from generate_ac3_container_priming_regressions import OUT, repeated_audio_edits


def generate():
    OUT.mkdir(parents=True, exist_ok=True)
    cases = []
    for ext in ["mkv", "mp4"]:
        path = OUT / ("tone." + ext)
        subprocess.run(
            [
                "ffmpeg",
                "-hide_banner",
                "-loglevel",
                "error",
                "-y",
                "-f",
                "lavfi",
                "-i",
                "color=c=gray:s=16x16:r=50:d=0.12",
                "-f",
                "lavfi",
                "-i",
                "aevalsrc=0.125*sin(2*PI*440*t):s=48000:d=0.12",
                "-map",
                "0:v:0",
                "-map",
                "1:a:0",
                "-c:v",
                "libvpx-vp9",
                "-lossless",
                "1",
                "-threads",
                "1",
                "-c:a",
                "ac3",
                "-ar",
                "48000",
                "-ac",
                "1",
                "-t",
                "0.12",
                *(["-movflags", "+faststart"] if ext == "mp4" else []),
                str(path),
            ],
            check=True,
        )
        ref = OUT / ("tone-" + ext + ".f32le")
        subprocess.run(
            [
                "ffmpeg",
                "-hide_banner",
                "-loglevel",
                "error",
                "-y",
                "-i",
                str(path),
                "-map",
                "0:a:0",
                "-vn",
                "-f",
                "f32le",
                "-c:a",
                "pcm_f32le",
                str(ref),
            ],
            check=True,
        )
        probe = json.loads(
            subprocess.check_output(
                [
                    "ffprobe",
                    "-v",
                    "error",
                    "-select_streams",
                    "a:0",
                    "-show_entries",
                    "stream=sample_rate,channels,duration,start_time:packet=pts,duration,side_data_list",
                    "-of",
                    "json",
                    str(path),
                ]
            )
        )
        cases.append(
            dict(
                file=path.name,
                reference=ref.name,
                reference_frames=ref.stat().st_size // 4,
                probe=probe,
            )
        )
    with tempfile.TemporaryDirectory(prefix="fvid-ac3-edits-") as directory:
        tail = Path(directory) / "tail.mp4"
        subprocess.run(
            [
                "ffmpeg",
                "-hide_banner",
                "-loglevel",
                "error",
                "-y",
                "-i",
                str(OUT / "tone.mp4"),
                "-map",
                "0",
                "-c",
                "copy",
                str(tail),
            ],
            check=True,
        )
        (OUT / "gap-repeat.mp4").write_bytes(repeated_audio_edits(tail.read_bytes()))
    files = list(OUT.glob("tone*")) + [OUT / "gap-repeat.mp4"]
    manifest = dict(
        provenance="Explicitly authored120ms16-square neutral VP9 plus mono440Hz tone, native AC3 in Matroska/MP4. FFmpeg is authoring/reference only; ordinary tests use saved files and no network/encoder. No private media.",
        cases=cases,
        sha256={p.name: hashlib.sha256(p.read_bytes()).hexdigest() for p in files},
    )
    (OUT / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")


if __name__ == "__main__":
    generate()
