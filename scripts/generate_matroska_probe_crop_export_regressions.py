#!/usr/bin/env python3
"""Purely author Matroska probe codec/metadata/disposition, container PixelCrop* decode/export, and canonical PCM codec regression fixtures."""
import hashlib, json, struct
from pathlib import Path
from generate_vp9_chroma import frame
from generate_audio_resample_window_fixture import ebml

OUT = Path(__file__).resolve().parents[1] / "tests/fixtures/playback-errors/matroska-probe-crop-export"


def simple_block(track_no, rel_ms, flags, payload):
    assert 1 <= track_no <= 126
    return ebml(
        "a3",
        bytes([0x80 | track_no])
        + struct.pack(">hB", rel_ms, flags)
        + payload,
    )


def simple_tag(name, value):
    return ebml(
        "67c8",
        ebml("45a3", name.encode("utf-8"))
        + ebml("4487", value.encode("utf-8")),
    )


def build_probe_parity_mkv():
    # Segment Info with TimestampScale=1_000_000 and WritingApp="FVidTestMuxer 1.0"
    header = ebml("1a45dfa3", ebml("4282", b"matroska"))
    info = ebml(
        "1549a966",
        ebml("2ad7b1", (1_000_000).to_bytes(3, "big"))
        + ebml("5741", b"FVidTestMuxer 1.0"),
    )
    # Track 1: A_PCM/INT/LIT 16-bit, TrackUID=101, FlagDefault=0, FlagForced=1, LanguageIETF="en-US"
    audio1 = (
        ebml("b5", struct.pack(">d", 48000.0))
        + ebml("9f", b"\x01")
        + ebml("6264", b"\x10")
    )
    track1 = ebml(
        "ae",
        ebml("d7", b"\x01")
        + ebml("73c5", (101).to_bytes(1, "big"))
        + ebml("83", b"\x02")
        + ebml("86", b"A_PCM/INT/LIT")
        + ebml("536e", b"Primary PCM")
        + ebml("22b59d", b"en-US")
        + ebml("88", b"\x00")
        + ebml("55aa", b"\x01")
        + ebml("e1", audio1),
    )
    # Track 2: A_PCM/FLOAT/IEEE 32-bit, TrackUID=202, no language element (defaults to "eng"), FlagDefault=1, FlagCommentary=1
    audio2 = (
        ebml("b5", struct.pack(">d", 48000.0))
        + ebml("9f", b"\x01")
        + ebml("6264", b"\x20")
    )
    track2 = ebml(
        "ae",
        ebml("d7", b"\x02")
        + ebml("73c5", (202).to_bytes(1, "big"))
        + ebml("83", b"\x02")
        + ebml("86", b"A_PCM/FLOAT/IEEE")
        + ebml("88", b"\x01")
        + ebml("55af", b"\x01")
        + ebml("e1", audio2),
    )
    # Track 3: A_ALAC, TrackUID=303
    alac_cookie = (
        b"\x00\x00\x00\x24alac\x00\x00\x00\x00"
        + struct.pack(">IBBBBBHIII", 4096, 0, 16, 40, 10, 14, 1, 255, 0, 0)[:-4]
        + struct.pack(">II", 0, 48000)
    )
    audio3 = ebml("b5", struct.pack(">d", 48000.0)) + ebml("9f", b"\x01")
    track3 = ebml(
        "ae",
        ebml("d7", b"\x03")
        + ebml("73c5", (303).to_bytes(2, "big"))
        + ebml("83", b"\x02")
        + ebml("86", b"A_ALAC")
        + ebml("63a2", alac_cookie)
        + ebml("e1", audio3),
    )
    # Track 4: A_MPEG/L1, TrackUID=404
    track4 = ebml(
        "ae",
        ebml("d7", b"\x04")
        + ebml("73c5", (404).to_bytes(2, "big"))
        + ebml("83", b"\x02")
        + ebml("86", b"A_MPEG/L1")
        + ebml("e1", audio3),
    )
    tracks = ebml("1654ae6b", track1 + track2 + track3 + track4)

    # Tags:
    # 1. File-level tag (empty Targets): title + custom "CUSTOM_FILE_KEY"
    tag_file = ebml(
        "7373",
        ebml("63c0", b"")
        + simple_tag("title", "Probe Parity Clip")
        + simple_tag("CUSTOM_FILE_KEY", "custom-file-val"),
    )
    # 2. All-tracks tag (TargetTrackUID = 0): "GLOBAL_TRACK_TAG"
    tag_global_track = ebml(
        "7373",
        ebml("63c0", ebml("63c5", b"\x00"))
        + simple_tag("GLOBAL_TRACK_TAG", "all-tracks"),
    )
    # 3. Track 1 scoped tag (TargetTrackUID = 101): "TRACK_ROLE"
    tag_track1 = ebml(
        "7373",
        ebml("63c0", ebml("63c5", (101).to_bytes(1, "big")))
        + simple_tag("TRACK_ROLE", "dialogue-stem"),
    )
    tags = ebml("1254c367", tag_file + tag_global_track + tag_track1)

    pcm16_samples = struct.pack("<4h", 0, 8192, -8192, 16384)
    pcmf32_samples = struct.pack("<4f", 0.0, 0.25, -0.25, 0.5)
    cluster = ebml(
        "1f43b675",
        ebml("e7", b"\x00")
        + simple_block(1, 0, 0x80, pcm16_samples)
        + simple_block(2, 0, 0x80, pcmf32_samples),
    )
    return header + ebml("18538067", info + tracks + tags + cluster)


def build_single_pcm16_mka():
    header = ebml("1a45dfa3", ebml("4282", b"matroska"))
    info = ebml("1549a966", ebml("2ad7b1", (1_000_000).to_bytes(3, "big")))
    audio = (
        ebml("b5", struct.pack(">d", 48000.0))
        + ebml("9f", b"\x01")
        + ebml("6264", b"\x10")
    )
    track = ebml(
        "ae",
        ebml("d7", b"\x01")
        + ebml("83", b"\x02")
        + ebml("86", b"A_PCM/INT/LIT")
        + ebml("e1", audio),
    )
    tracks = ebml("1654ae6b", track)
    pcm16_samples = struct.pack("<4h", 0, 8192, -8192, 16384)
    cluster = ebml(
        "1f43b675",
        ebml("e7", b"\x00") + simple_block(1, 0, 0x80, pcm16_samples),
    )
    return header + ebml("18538067", info + tracks + cluster)


def build_cropped_vp9_mkv():
    # 16x16 coded VP9 4:2:0 frame with PixelCropLeft=2, PixelCropRight=2, PixelCropTop=4, PixelCropBottom=4
    # -> visible dimensions = 12x8!
    # Also set DisplayWidth=24, DisplayHeight=8 so pixel_aspect = (24*8, 8*12) = (2, 1).
    vp9_kf = frame(8, "420", 16, 16)
    header = ebml("1a45dfa3", ebml("4282", b"matroska"))
    info = ebml("1549a966", ebml("2ad7b1", (1_000_000).to_bytes(3, "big")))
    video = (
        ebml("b0", bytes([16]))
        + ebml("ba", bytes([16]))
        + ebml("54cc", bytes([2]))  # PixelCropLeft = 2
        + ebml("54dd", bytes([2]))  # PixelCropRight = 2
        + ebml("54bb", bytes([4]))  # PixelCropTop = 4
        + ebml("54aa", bytes([4]))  # PixelCropBottom = 4
        + ebml("54b0", bytes([24]))  # DisplayWidth = 24
        + ebml("54ba", bytes([8]))  # DisplayHeight = 8
    )
    track = ebml(
        "ae",
        ebml("d7", b"\x01")
        + ebml("83", b"\x01")
        + ebml("86", b"V_VP9")
        + ebml("23e383", (40_000_000).to_bytes(4, "big"))
        + ebml("e0", video),
    )
    tracks = ebml("1654ae6b", track)
    cluster = ebml(
        "1f43b675",
        ebml("e7", b"\x00")
        + simple_block(1, 0, 0x80, vp9_kf)
        + simple_block(1, 40, 0x80, vp9_kf),
    )
    return header + ebml("18538067", info + tracks + cluster)


def generate():
    files = {
        "matroska-probe-parity.mkv": build_probe_parity_mkv(),
        "matroska-pcm-s16le.mka": build_single_pcm16_mka(),
        "matroska-cropped-16x16-to-12x8.mkv": build_cropped_vp9_mkv(),
    }
    manifest = {
        "generator": "scripts/generate_matroska_probe_crop_export_regressions.py",
        "files": {
            name: {
                "bytes": len(data),
                "sha256": hashlib.sha256(data).hexdigest(),
            }
            for name, data in sorted(files.items())
        },
    }
    return files, manifest


def write_all():
    OUT.mkdir(parents=True, exist_ok=True)
    files, manifest = generate()
    for name, data in files.items():
        (OUT / name).write_bytes(data)
    (OUT / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")


if __name__ == "__main__":
    write_all()
