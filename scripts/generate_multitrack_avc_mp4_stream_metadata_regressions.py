#!/usr/bin/env python3
"""Generate deterministic synthetic fixtures for Batch 233 (`FVID-0240`).

Covers:
1. Multitrack Matroska with `V_MPEG4/ISO/AVC` video + `A_OPUS` audio
   (`avc-opus-multitrack.mkv`), with container `WritingApp` (`"FVidAvcMultitrackMux"`),
   Track 1 language `"fra"` (`LanguageIETF`), and Track 2 with no explicit
   language element (Matroska default `"eng"`).
2. Three-track MP4 (`avc-dual-aac-tracks.mp4`) with 16x16 `avc1` I-PCM video
   (track 0) and two `mp4a` AAC audio tracks (tracks 1 and 2) to verify
   `owned_mp4_multitrack_export` stream selection (`[0, 2]`, `[2, 0]`) and
   per-track metadata mutation (`stream_metadata_set`, `stream_metadata_delete`).
"""

from __future__ import annotations

import hashlib
import json
from pathlib import Path
import struct

from generate_audio_resample_window_fixture import ebml
from generate_avc_mbaff_direct_samples import Writer
from generate_mp4_audio_selection_regressions import (
    build_aac_packet,
    build_aac_pulse_packet,
    build_esds,
    make_audio_track,
    mp4a_v0_header,
)
from generate_vp9_chroma import atom, word


ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / "tests/fixtures/playback-errors/multitrack-avc-mp4-stream-metadata"


def u(tag: str, n: int) -> bytes:
    return ebml(tag, n.to_bytes(max(1, (n.bit_length() + 7) // 8), "big"))


def f64_elem(tag: str, val: float) -> bytes:
    return ebml(tag, struct.pack(">d", val))


def simple_tag(name: str, value: str) -> bytes:
    return ebml(
        "67c8",
        ebml("45a3", name.encode("utf-8")) + ebml("4487", value.encode("utf-8")),
    )


def build_avc_16x16_ipcm() -> tuple[bytes, list[bytes]]:
    """Build 16x16 4:2:0 8-bit progressive AVC avcC and 2 I_PCM frames."""
    b = Writer()
    for val in [244, 0, 30]:
        b.u(val, 8)
    b.ue(0)  # seq_parameter_set_id = 0
    b.ue(1)  # chroma_format_idc = 1 (4:2:0)
    b.ue(0)  # bit_depth_luma_minus8 = 0
    b.ue(0)  # bit_depth_chroma_minus8 = 0
    b.u(0)   # qpprime_y_zero_transform_bypass_flag = 0
    b.u(0)   # seq_scaling_matrix_present_flag = 0
    b.ue(0)  # log2_max_frame_num_minus4 = 0
    b.ue(0)  # pic_order_cnt_type = 0
    b.ue(0)  # log2_max_pic_order_cnt_lsb_minus4 = 0
    b.ue(1)  # max_num_ref_frames = 1
    b.u(0)   # gaps_in_frame_num_value_allowed_flag = 0
    b.ue(0)  # pic_width_in_mbs_minus1 = 0 (16 px)
    b.ue(0)  # pic_height_in_map_units_minus1 = 0 (16 px)
    b.u(1)   # frame_mbs_only_flag = 1
    b.u(1)   # direct_8x8_inference_flag = 1
    b.u(0)   # frame_cropping_flag = 0
    b.u(0)   # vui_parameters_present_flag = 0
    sps = b.nal(0x67)

    p = Writer()
    p.ue(0)  # pic_parameter_set_id = 0
    p.ue(0)  # seq_parameter_set_id = 0
    p.u(0)   # entropy_coding_mode_flag = 0 (CAVLC)
    p.u(0)   # bottom_field_pic_order_in_frame_present_flag = 0
    p.ue(0)  # num_slice_groups_minus1 = 0
    p.ue(0)  # num_ref_idx_l0_default_active_minus1 = 0
    p.ue(0)  # num_ref_idx_l1_default_active_minus1 = 0
    p.u(0)   # weighted_pred_flag = 0
    p.u(0, 2)  # weighted_bipred_idc = 0
    p.se(0)  # pic_init_qp_minus26 = 0
    p.se(0)  # pic_init_qs_minus26 = 0
    p.se(0)  # chroma_qp_index_offset = 0
    p.u(1)   # deblocking_filter_control_present_flag = 1
    p.u(0)   # constrained_intra_pred_flag = 0
    p.u(0)   # redundant_pic_cnt_present_flag = 0
    pps = p.nal(0x68)

    avcc = (
        bytes([1, 244, 0, 30, 255, 225])
        + len(sps).to_bytes(2, "big")
        + sps
        + bytes([1])
        + len(pps).to_bytes(2, "big")
        + pps
    )

    packets = []
    for frame_idx in range(2):
        s = Writer()
        s.ue(0)  # first_mb_in_slice = 0
        s.ue(2)  # slice_type = I (2)
        s.ue(0)  # pic_parameter_set_id = 0
        s.u(frame_idx, 4)  # frame_num
        if frame_idx == 0:
            s.ue(0)  # idr_pic_id = 0
        s.u(frame_idx * 2, 4)  # pic_order_cnt_lsb
        if frame_idx == 0:
            s.u(0)  # no_output_of_prior_pics_flag = 0
            s.u(0)  # long_term_reference_flag = 0
        else:
            s.u(0)  # adaptive_ref_pic_marking_mode_flag = 0
        s.se(0)  # slice_qp_delta = 0
        s.ue(1)  # disable_deblocking_filter_idc = 1
        s.ue(25)  # mb_type = I_PCM (25)
        s.align()
        for y in range(16):
            for x in range(16):
                s.u(40 + frame_idx * 30 + (x + y) % 32, 8)
        for _ in range(64):
            s.u(120 + frame_idx * 4, 8)
        for _ in range(64):
            s.u(136 - frame_idx * 4, 8)
        nal = s.nal(0x65 if frame_idx == 0 else 0x41)
        packets.append(len(nal).to_bytes(4, "big") + nal)
    return avcc, packets


def build_avc_opus_multitrack_mkv(avcc: bytes, avc_packets: list[bytes]) -> bytes:
    header = ebml(
        "1a45dfa3",
        ebml("4282", b"matroska") + u("4287", 4) + u("4285", 2),
    )
    info = ebml(
        "1549a966",
        u("2ad7b1", 1_000_000)
        + f64_elem("4489", 40.0)
        + ebml("4d80", b"FVidAvcMultitrackMux")
        + ebml("5741", b"FVidAvcMultitrackApp"),
    )
    video_track = ebml(
        "ae",
        u("d7", 1)
        + u("73c5", 101)
        + u("83", 1)
        + ebml("86", b"V_MPEG4/ISO/AVC")
        + ebml("63a2", avcc)
        + u("23e383", 20_000_000)
        + ebml("536e", b"PrimaryAvc")
        + ebml("22b59d", b"fr-CA")
        + ebml("22b59c", b"fra")
        + ebml("e0", u("b0", 16) + u("ba", 16)),
    )
    opus_head = b"OpusHead" + bytes([1, 1]) + struct.pack("<HIhB", 0, 48000, 0, 0)
    # Note: no explicit language element on track 2 so Matroska default "eng" applies.
    audio_track = ebml(
        "ae",
        u("d7", 2)
        + u("73c5", 202)
        + u("83", 2)
        + ebml("86", b"A_OPUS")
        + ebml("63a2", opus_head)
        + u("56aa", 0)
        + u("23e383", 20_000_000)
        + ebml("536e", b"CompanionOpus")
        + ebml("e1", f64_elem("b5", 48000.0) + u("9f", 1)),
    )
    tracks = ebml("1654ae6b", video_track + audio_track)
    file_tag = ebml(
        "7373",
        ebml("63c0", b"")
        + simple_tag("TITLE", "AVC Opus Multitrack")
        + simple_tag("SOURCE_TAG", "FromAvcMkv"),
    )
    track1_tag = ebml(
        "7373",
        ebml("63c0", u("63c5", 101))
        + simple_tag("VIDEO_ROLE", "main-camera"),
    )
    track2_tag = ebml(
        "7373",
        ebml("63c0", u("63c5", 202))
        + simple_tag("AUDIO_ROLE", "boom-mic"),
    )
    tags = ebml("1254c367", file_tag + track1_tag + track2_tag)

    opus_pkt = bytes.fromhex("f8fffe")
    cluster = u("e7", 0)
    for idx, v_pkt in enumerate(avc_packets):
        ts = idx * 20
        cluster += ebml("a3", b"\x81" + ts.to_bytes(2, "big") + b"\x80" + v_pkt)
        cluster += ebml("a3", b"\x82" + ts.to_bytes(2, "big") + b"\x80" + opus_pkt)

    return header + ebml("18538067", info + tracks + tags + ebml("1f43b675", cluster))


def build_avc_video_track_mp4(
    track_id: int,
    avcc: bytes,
    packets: list[bytes],
    offset: int,
    rate: int = 48000,
    delta: int = 1024,
) -> bytes:
    entry = bytearray(78)
    entry[6:8] = struct.pack(">H", 1)
    entry[24:28] = struct.pack(">HH", 16, 16)
    entry[28:36] = word(72 << 16, 72 << 16)
    entry[40:42] = struct.pack(">H", 1)
    entry[74:78] = struct.pack(">HH", 24, 65535)
    avc1_box = atom(b"avc1", bytes(entry) + atom(b"avcC", avcc))
    stbl = (
        atom(b"stsd", word(0, 1) + avc1_box)
        + atom(b"stts", word(0, 1, len(packets), delta))
        + atom(b"stsc", word(0, 1, 1, len(packets), 1))
        + atom(b"stsz", word(0, 0, len(packets), *map(len, packets)))
        + atom(b"stco", word(0, 1, offset))
        + atom(b"stss", word(0, 2, 1, 2))
    )
    dinf = atom(b"dinf", atom(b"dref", word(0, 1) + atom(b"url ", word(1))))
    vmhd = atom(b"vmhd", word(1, 0, 0))
    mdhd = bytearray(24)
    mdhd[12:20] = word(rate, len(packets) * delta)
    # Language "eng" (0x15c7)
    struct.pack_into(">H", mdhd, 20, 0x15C7)
    mdia = atom(
        b"mdia",
        atom(b"mdhd", bytes(mdhd))
        + atom(b"hdlr", word(0, 0) + b"vide" + bytes(12) + b"VideoHandler\x00")
        + atom(b"minf", vmhd + dinf + atom(b"stbl", stbl)),
    )
    tkhd = bytearray(84)
    tkhd[:4] = word(3)
    tkhd[12:16] = word(track_id)
    tkhd[20:24] = word(len(packets) * delta)
    tkhd[40:44] = tkhd[56:60] = word(65536)
    tkhd[72:76] = word(0x40000000)
    tkhd[76:84] = word(16 << 16, 16 << 16)
    udta = atom(b"udta", atom(b"name", b"SourceVideo"))
    return atom(b"trak", atom(b"tkhd", bytes(tkhd)) + mdia + udta)


def build_avc_dual_aac_mp4(avcc: bytes, avc_packets: list[bytes]) -> bytes:
    ftyp = atom(b"ftyp", b"isom\x00\x00\x00\x00isomiso6avc1mp41")
    aac_1 = [build_aac_packet(100), build_aac_packet(100)]
    aac_2 = [build_aac_pulse_packet(100, 1, 8), build_aac_packet(100)]
    v_payload = b"".join(avc_packets)
    a1_payload = b"".join(aac_1)
    a2_payload = b"".join(aac_2)
    mdat = atom(b"mdat", v_payload + a1_payload + a2_payload)

    v_offset = len(ftyp) + 8
    a1_offset = v_offset + len(v_payload)
    a2_offset = a1_offset + len(a1_payload)

    asc = b"\x11\x88"
    mp4a_payload = mp4a_v0_header(1, 48000) + atom(b"esds", build_esds(asc))

    v_trak = build_avc_video_track_mp4(1, avcc, avc_packets, v_offset, 48000, 1024)
    a1_trak = make_audio_track(2, 3, b"mp4a", mp4a_payload, aac_1, a1_offset, 48000, 1024)
    a2_trak = make_audio_track(3, 3, b"mp4a", mp4a_payload, aac_2, a2_offset, 48000, 1024)

    mvhd = bytearray(100)
    mvhd[12:16] = word(48000)
    mvhd[16:20] = word(2048)
    mvhd[20:24] = word(65536)
    mvhd[24:28] = word(0x01000000)
    for at, val in [(36, 65536), (52, 65536), (68, 1 << 30), (96, 4)]:
        mvhd[at : at + 4] = word(val)

    return ftyp + mdat + atom(b"moov", atom(b"mvhd", bytes(mvhd)) + v_trak + a1_trak + a2_trak)


def generate() -> dict[str, bytes]:
    avcc, avc_packets = build_avc_16x16_ipcm()
    files = {
        "avc-opus-multitrack.mkv": build_avc_opus_multitrack_mkv(avcc, avc_packets),
        "avc-dual-aac-tracks.mp4": build_avc_dual_aac_mp4(avcc, avc_packets),
    }
    manifest = {
        "batch_id": "FVID-0240",
        "generator": "scripts/generate_multitrack_avc_mp4_stream_metadata_regressions.py",
        "files": {
            name: {
                "bytes": len(data),
                "sha256": hashlib.sha256(data).hexdigest(),
            }
            for name, data in sorted(files.items())
        },
    }
    files["manifest.json"] = (json.dumps(manifest, indent=2, sort_keys=True) + "\n").encode("utf-8")
    return files


def main() -> None:
    OUT.mkdir(parents=True, exist_ok=True)
    for name, data in generate().items():
        (OUT / name).write_bytes(data)


if __name__ == "__main__":
    main()
