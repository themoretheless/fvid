#!/usr/bin/env python3
"""Purely author Matroska/WebM BlockGroup validation, Block/SimpleBlock reserved-flag, and equal-PTS seek regression fixtures."""
import hashlib, json, struct
from pathlib import Path
from generate_vp9_chroma import coloured, frame
from generate_audio_resample_window_fixture import ebml

OUT = Path(__file__).resolve().parents[1] / "tests/fixtures/playback-errors/webm-block-seek"


def vp9_non_shown_keyframe(depth, chroma, side=16, dc_quant=4):
    raw_kf, expected = coloured(depth, chroma, side=side, dc_quant=dc_quant)
    b0 = raw_kf[0]
    assert (b0 & 0x0E) == 0x02  # keyframe=0, show_frame=1, error_resilient=0
    non_shown = bytes([b0 & ~0x02]) + raw_kf[1:]
    return non_shown, expected


def vp9_show_existing_frame(slot=0):
    # frame_marker=2 (10), profile=0 (00), show_existing_frame=1 (1), frame_to_show_map_idx=slot (3 bits)
    return bytes([0x88 | (slot & 7)])


def build_webm_doc(doctype, tracks_payload, clusters_payload, scale_ns=1_000_000):
    header = ebml("1a45dfa3", ebml("4282", doctype))
    info = ebml("1549a966", ebml("2ad7b1", scale_ns.to_bytes(3, "big")))
    tracks = ebml("1654ae6b", tracks_payload)
    return header + ebml("18538067", info + tracks + clusters_payload)


def vp9_track_entry(track_no=1, side=16, default_duration_ns=40_000_000):
    video = ebml("b0", bytes([side])) + ebml("ba", bytes([side]))
    return ebml(
        "ae",
        ebml("d7", bytes([track_no]))
        + ebml("83", b"\x01")
        + ebml("86", b"V_VP9")
        + ebml("23e383", default_duration_ns.to_bytes(4, "big"))
        + ebml("e0", video),
    )


def pcm_track_entry(track_no=1, rate=48000, channels=1, bits=16, codec_delay_ns=0):
    audio = (
        ebml("b5", struct.pack(">d", rate))
        + ebml("9f", bytes([channels]))
        + ebml("6264", bytes([bits]))
    )
    body = (
        ebml("d7", bytes([track_no]))
        + ebml("83", b"\x02")
        + ebml("86", b"A_PCM/INT/LIT")
        + ebml("e1", audio)
    )
    if codec_delay_ns:
        body += ebml("56aa", codec_delay_ns.to_bytes(4, "big"))
    return ebml("ae", body)


def simple_block(track_no, rel_ms, flags, payload):
    assert 1 <= track_no <= 126
    return ebml(
        "a3",
        bytes([0x80 | track_no])
        + struct.pack(">hB", rel_ms, flags)
        + payload,
    )


def raw_block(track_no, rel_ms, flags, payload):
    assert 1 <= track_no <= 126
    return ebml(
        "a1",
        bytes([0x80 | track_no])
        + struct.pack(">hB", rel_ms, flags)
        + payload,
    )


def generate():
    kf_non_shown, expected_planes = vp9_non_shown_keyframe(8, "420", side=16, dc_quant=4)
    show_existing = vp9_show_existing_frame(0)
    plain_kf = frame(8, "420", 16, 16)

    files = {
        "expected-16x16.yuv420p16le": bytes(expected_planes),
    }
    controls = []
    cases = []

    # Control 1: Equal-PTS VP9 keyframe + show-existing keyframe in single cluster,
    # plus an initial shown keyframe at 0 ms so seeking to 40 ms exercises equal-PTS tie-breaking.
    cluster_equal_pts = ebml(
        "1f43b675",
        ebml("e7", (0).to_bytes(1, "big"))
        + simple_block(1, 0, 0x80, plain_kf)
        + simple_block(1, 40, 0x88, kf_non_shown)  # keyframe + invisible at 40 ms
        + simple_block(1, 40, 0x80, show_existing),  # keyframe + visible show-existing at 40 ms
    )
    files["valid-equal-pts-vp9-seek.webm"] = build_webm_doc(
        b"webm", vp9_track_entry(1, 16), cluster_equal_pts
    )
    controls.append(
        dict(
            file="valid-equal-pts-vp9-seek.webm",
            mode="equal-pts-invisible-keyframe-before-show-existing",
        )
    )

    # Control 2: Valid BlockGroup with BlockDuration and single Block (flags=0x00)
    cluster_valid_bg = ebml(
        "1f43b675",
        ebml("e7", b"\x00")
        + ebml(
            "a0",
            raw_block(1, 0, 0x00, plain_kf)
            + ebml("9b", (40).to_bytes(1, "big")),
        ),
    )
    files["valid-blockgroup.mkv"] = build_webm_doc(
        b"matroska", vp9_track_entry(1, 16), cluster_valid_bg
    )
    controls.append(
        dict(file="valid-blockgroup.mkv", mode="single-block-in-blockgroup")
    )

    # Control 3: Multi-track Matroska with video track 1 at 0 ms and audio track 2 starting at 50 ms
    # plus a second audio packet sharing timestamp 50 ms, to test WebmAudioReader::seek pre-origin and equal-PTS.
    pcm_pkt_a = struct.pack("<4h", 1000, 2000, 3000, 4000)
    pcm_pkt_b = struct.pack("<4h", 5000, 6000, 7000, 8000)
    cluster_multitrack = (
        ebml(
            "1f43b675",
            ebml("e7", b"\x00") + simple_block(1, 0, 0x80, plain_kf),
        )
        + ebml(
            "1f43b675",
            ebml("e7", (50).to_bytes(1, "big"))
            + simple_block(2, 0, 0x80, pcm_pkt_a)
            + simple_block(2, 0, 0x80, pcm_pkt_b),
        )
    )
    files["valid-multitrack-audio-seek.mkv"] = build_webm_doc(
        b"matroska",
        vp9_track_entry(1, 16) + pcm_track_entry(2, 48000, 1, 16),
        cluster_multitrack,
    )
    controls.append(
        dict(
            file="valid-multitrack-audio-seek.mkv",
            mode="multitrack-delayed-audio-origin-and-equal-pts",
        )
    )

    # Invalid 1: BlockGroup lacking a Block (0xa1) element
    cluster_missing_block = ebml(
        "1f43b675",
        ebml("e7", b"\x00")
        + ebml("a0", ebml("9b", (40).to_bytes(1, "big"))),
    )
    files["blockgroup-missing-block.mkv"] = build_webm_doc(
        b"matroska", vp9_track_entry(1, 16), cluster_missing_block
    )
    cases.append(
        dict(
            file="blockgroup-missing-block.mkv",
            expected_error="Matroska BlockGroup lacks Block",
        )
    )

    # Invalid 2: BlockGroup with duplicate Block (0xa1) elements
    cluster_dup_block = ebml(
        "1f43b675",
        ebml("e7", b"\x00")
        + ebml(
            "a0",
            raw_block(1, 0, 0x00, plain_kf)
            + raw_block(1, 40, 0x00, plain_kf),
        ),
    )
    files["blockgroup-duplicate-block.mkv"] = build_webm_doc(
        b"matroska", vp9_track_entry(1, 16), cluster_dup_block
    )
    cases.append(
        dict(
            file="blockgroup-duplicate-block.mkv",
            expected_error="duplicate Matroska Block in BlockGroup",
        )
    )

    # Invalid 3: SimpleBlock with non-zero reserved flag bits (0x70, e.g. 0x90)
    cluster_sb_reserved = ebml(
        "1f43b675",
        ebml("e7", b"\x00") + simple_block(1, 0, 0x90, plain_kf),
    )
    files["simpleblock-reserved-flags.webm"] = build_webm_doc(
        b"webm", vp9_track_entry(1, 16), cluster_sb_reserved
    )
    cases.append(
        dict(
            file="simpleblock-reserved-flags.webm",
            expected_error="invalid Matroska block flags",
        )
    )

    # Invalid 4: Block (0xa1) inside BlockGroup with bit 0x01 (SimpleBlock-only Discardable) set
    cluster_block_bit01 = ebml(
        "1f43b675",
        ebml("e7", b"\x00")
        + ebml("a0", raw_block(1, 0, 0x01, plain_kf)),
    )
    files["blockgroup-block-discardable-bit.mkv"] = build_webm_doc(
        b"matroska", vp9_track_entry(1, 16), cluster_block_bit01
    )
    cases.append(
        dict(
            file="blockgroup-block-discardable-bit.mkv",
            expected_error="invalid Matroska block flags",
        )
    )

    manifest = dict(
        issue="Matroska/WebM BlockGroup cardinality, Block/SimpleBlock reserved header flag bits, and equal-PTS/pre-origin seek indexing",
        controls=[
            c
            | dict(
                size=len(files[c["file"]]),
                sha256=hashlib.sha256(files[c["file"]]).hexdigest(),
            )
            for c in controls
        ],
        cases=[
            c
            | dict(
                size=len(files[c["file"]]),
                sha256=hashlib.sha256(files[c["file"]]).hexdigest(),
            )
            for c in cases
        ],
    )
    files["manifest.json"] = (json.dumps(manifest, indent=2) + "\n").encode()
    return files


def main():
    OUT.mkdir(parents=True, exist_ok=True)
    for name, data in generate().items():
        (OUT / name).write_bytes(data)


if __name__ == "__main__":
    main()
