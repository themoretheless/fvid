"""Author synthetic E-AC-3 playback regression fixtures and reference PCM (FFmpeg reference benchmark)."""

import hashlib
import json
import struct
import subprocess

from generate_eac3_playback import (
    OUT,
    build_blksw_frame,
    build_spx_atten_frame,
    mutate_reduced_rate,
)


def generate() -> dict[str, bytes]:
    OUT.mkdir(parents=True, exist_ok=True)
    files: dict[str, bytes] = {}
    cases = []

    # 1. Container tracks (MP4 ec-3 and Matroska A_EAC3) with 6-block E-AC-3 (48 kHz stereo)
    for ext in ["mp4", "mkv"]:
        path = OUT / f"stereo-48k.{ext}"
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
                "color=c=gray:s=16x16:r=50:d=0.064",
                "-f",
                "lavfi",
                "-i",
                "aevalsrc=0.25*sin(2*PI*440*t)|0.2*sin(2*PI*660*t):s=48000:d=0.064",
                "-map",
                "0:v:0",
                "-map",
                "1:a:0",
                "-c:v",
                "libvpx-vp9" if ext == "mkv" else "libx264",
                *(["-lossless", "1"] if ext == "mkv" else ["-crf", "0", "-pix_fmt", "yuv420p"]),
                "-threads",
                "1",
                "-c:a",
                "eac3",
                "-b:a",
                "192k",
                "-t",
                "0.064",
                *(["-movflags", "+faststart"] if ext == "mp4" else []),
                str(path),
            ],
            check=True,
        )
        files[path.name] = path.read_bytes()

    # Container 5.1 E-AC-3 (MP4 and MKV)
    for ext in ["mp4", "mkv"]:
        path = OUT / f"surround-51.{ext}"
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
                "aevalsrc=0.3*sin(2*PI*440*t)|0.25*sin(2*PI*550*t)|0.2*sin(2*PI*660*t)|0.3*sin(2*PI*80*t)|0.15*sin(2*PI*880*t)|0.1*sin(2*PI*1100*t):s=48000:d=0.064",
                "-vn",
                "-ch_layout",
                "5.1",
                "-c:a",
                "eac3",
                "-b:a",
                "448k",
                "-t",
                "0.064",
                *(["-movflags", "+faststart"] if ext == "mp4" else []),
                str(path),
            ],
            check=True,
        )
        files[path.name] = path.read_bytes()

    # Container with 2-block syncframes (numblkscod = 1) in MKV and MP4
    for ext in ["mp4", "mkv"]:
        path = OUT / f"blocks-2.{ext}"
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
                "sine=frequency=440:sample_rate=48000:duration=0.032",
                "-vn",
                "-c:a",
                "eac3",
                "-b:a",
                "3024k",
                *(["-movflags", "+faststart", "-use_editlist", "0"] if ext == "mp4" else []),
                str(path),
            ],
            check=True,
        )
        files[path.name] = path.read_bytes()

    # 2. Elementary .ec3 streams for 1, 2, 3, and 6 blocks per syncframe
    for blocks, bitrate in [(6, "192k"), (3, "1536k"), (2, "3024k"), (1, "6144k")]:
        name = f"blocks-{blocks}.ec3"
        path = OUT / name
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
                "aevalsrc=0.25*sin(2*PI*440*t)|0.2*sin(2*PI*660*t):s=48000:d=0.064",
                "-c:a",
                "eac3",
                "-b:a",
                bitrate,
                str(path),
            ],
            check=True,
        )
        raw = path.read_bytes()
        numblkscod = (raw[4] >> 4) & 3
        assert [1, 2, 3, 6][numblkscod] == blocks
        ref = subprocess.check_output(
            ["ffmpeg", "-v", "error", "-i", str(path), "-f", "f32le", "-"]
        )
        ref_name = f"blocks-{blocks}.f32le"
        (OUT / ref_name).write_bytes(ref)
        files[name] = raw
        files[ref_name] = ref
        cases.append(
            {
                "name": f"blocks-{blocks}",
                "file": name,
                "reference": ref_name,
                "blocks": blocks,
                "sample_rate": 48000,
                "channels": 2,
                "pcm_frames": len(ref) // 8,
            }
        )

    # 3. Reduced sample rates (fscod = 3, fscod2 = 0 -> 24000, 1 -> 22050, 2 -> 16000)
    # Derived from a 2-frame (6 blocks each) stereo stream at matching base rate (48000, 44100, 32000 Hz)
    # Header probes only: packed mantissas are NOT reallocated for half rate.
    # Valid owned PCM acceptance is authored by generate_eac3_reduced_allocation.py.
    for fscod2, base_rate, rate in [(0, 48000, 24000), (1, 44100, 22050), (2, 32000, 16000)]:
        name = f"reduced-{rate}.ec3"
        base_raw = subprocess.check_output(
            [
                "ffmpeg",
                "-hide_banner",
                "-loglevel",
                "error",
                "-f",
                "lavfi",
                "-i",
                f"aevalsrc=0.25*sin(2*PI*440*t)|0.2*sin(2*PI*660*t):s={base_rate}:d=0.064",
                "-frames:a",
                "2",
                "-c:a",
                "eac3",
                "-b:a",
                "192k",
                "-f",
                "eac3",
                "-",
            ]
        )
        data = mutate_reduced_rate(base_raw, fscod2, fscod2)
        (OUT / name).write_bytes(data)
        files[name] = data
        cases.append(
            {
                "name": f"reduced-{rate}",
                "file": name,
                "blocks": 6,
                "sample_rate": rate,
                "channels": 2,
                "pcm_frames": 2 * 1536,
            }
        )

    # 4. Block switching (blkswe = 1) and Spectral Extension boundary attenuation (spxattene = 1)
    blksw_raw = build_blksw_frame([(1, 0), (1, 1), (0, 1), (1, 0), (0, 0), (1, 1)], absexp=8) * 2
    blksw_ref = subprocess.run(
        ["ffmpeg", "-v", "error", "-f", "eac3", "-i", "pipe:0", "-f", "f32le", "-"],
        input=blksw_raw,
        capture_output=True,
        check=True,
    ).stdout
    (OUT / "blksw.ec3").write_bytes(blksw_raw)
    (OUT / "blksw.f32le").write_bytes(blksw_ref)
    files["blksw.ec3"] = blksw_raw
    files["blksw.f32le"] = blksw_ref
    cases.append(
        {
            "name": "blksw",
            "file": "blksw.ec3",
            "reference": "blksw.f32le",
            "blocks": 6,
            "sample_rate": 48000,
            "channels": 2,
            "pcm_frames": len(blksw_ref) // 8,
        }
    )

    spx_no_atten_raw = build_spx_atten_frame(None) * 2
    spx_no_atten_ref = subprocess.run(
        ["ffmpeg", "-v", "error", "-f", "eac3", "-i", "pipe:0", "-f", "f32le", "-"],
        input=spx_no_atten_raw,
        capture_output=True,
        check=True,
    ).stdout
    (OUT / "spx-no-atten.ec3").write_bytes(spx_no_atten_raw)
    (OUT / "spx-no-atten.f32le").write_bytes(spx_no_atten_ref)
    files["spx-no-atten.ec3"] = spx_no_atten_raw
    files["spx-no-atten.f32le"] = spx_no_atten_ref
    cases.append(
        {
            "name": "spx-no-atten",
            "file": "spx-no-atten.ec3",
            "reference": "spx-no-atten.f32le",
            "blocks": 6,
            "sample_rate": 48000,
            "channels": 2,
            "pcm_frames": len(spx_no_atten_ref) // 8,
        }
    )

    spx_atten_raw = build_spx_atten_frame([15, 31]) * 2
    spx_atten_ref = subprocess.run(
        ["ffmpeg", "-v", "error", "-f", "eac3", "-i", "pipe:0", "-f", "f32le", "-"],
        input=spx_atten_raw,
        capture_output=True,
        check=True,
    ).stdout
    (OUT / "spx-atten.ec3").write_bytes(spx_atten_raw)
    (OUT / "spx-atten.f32le").write_bytes(spx_atten_ref)
    files["spx-atten.ec3"] = spx_atten_raw
    files["spx-atten.f32le"] = spx_atten_ref
    cases.append(
        {
            "name": "spx-atten",
            "file": "spx-atten.ec3",
            "reference": "spx-atten.f32le",
            "blocks": 6,
            "sample_rate": 48000,
            "channels": 2,
            "pcm_frames": len(spx_atten_ref) // 8,
        }
    )

    # 5. Container rate mismatch control & reproducer (48 kHz E-AC-3 labeled as 32 kHz)
    mkv_48 = files["stereo-48k.mkv"]
    old_rate = b"\xb5\x88" + struct.pack(">d", 48000.0)
    new_rate = b"\xb5\x88" + struct.pack(">d", 32000.0)
    assert mkv_48.count(old_rate) == 1
    mismatch_mkv = mkv_48.replace(old_rate, new_rate)
    (OUT / "mismatch-rate.mkv").write_bytes(mismatch_mkv)
    files["mismatch-rate.mkv"] = mismatch_mkv

    mp4_48 = bytearray(files["stereo-48k.mp4"])
    mdhd_at = mp4_48.index(b"mdhd\x00\x00\x00\x00", mp4_48.index(b"soun") - 200)
    assert struct.unpack_from(">I", mp4_48, mdhd_at + 16)[0] == 48000
    struct.pack_into(">I", mp4_48, mdhd_at + 16, 32000)
    at = mp4_48.index(b"ec-3") + 28
    assert struct.unpack_from(">I", mp4_48, at)[0] == 48000 << 16
    struct.pack_into(">I", mp4_48, at, 32000 << 16)
    mismatch_mp4 = bytes(mp4_48)
    (OUT / "mismatch-rate.mp4").write_bytes(mismatch_mp4)
    files["mismatch-rate.mp4"] = mismatch_mp4

    manifest = {
        "provenance": (
            "Synthetic E-AC-3 (Dolby Digital Plus) regression fixtures covering MP4 (ec-3), "
            "Matroska (A_EAC3), elementary .ec3 streams, 1/2/3/6-block syncframes (numblkscod 0..3), "
            "and reduced sample rates (fscod=3, fscod2=0..2: 24000, 22050, 16000 Hz). "
            "FFmpeg is used only during offline fixture authoring; ordinary tests load saved files."
        ),
        "cases": cases,
        "sha256": {k: hashlib.sha256(v).hexdigest() for k, v in sorted(files.items())},
    }
    manifest_bytes = (json.dumps(manifest, indent=2) + "\n").encode()
    (OUT / "manifest.json").write_bytes(manifest_bytes)
    files["manifest.json"] = manifest_bytes
    return files


if __name__ == "__main__":
    generate()
