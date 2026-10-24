#!/usr/bin/env python3
"""Generate deterministic synthetic fixtures for Batch 235 (`FVID-0242`).

Covers:
1. Rotated MP4 video (`tkhd` rotation 90 and 180) in `owned_mp4_video_decode`,
   `owned_mp4_video_bridge`, and `owned_mp4_multitrack_export`:
   - `avc-rotated-90-anamorphic.mp4`: 32x16 coded `avc1` I_PCM with `tkhd`
     rotation 90 (display 16x32) and `pasp = (2, 1)` (`track.pixel_aspect = (2, 1)`),
     so `fvid_media::decode_video` and `fvid_media::decode_video_transformed`
     report display dimensions `16x32` and `fvid_media::transcode_lossless`
     exports upright `16x32` FFV1 frames with `rotation == 0` and preserved
     post-rotation `pixel_aspect == (2, 1)`.
   - `avc-rotated-90-multitrack-aac.mp4`: 32x16 coded `avc1` I_PCM with `tkhd`
     rotation 90 + 2 AAC companion tracks (2 packets of 1024 samples at 48000 Hz =
     2048 samples per track), verifying `owned_mp4_multitrack_export` supports
     rotated MP4 video, stream selection, and interval trimming (`trimmed_audio_sample_frames`).
2. Single-track MP4 disposition (`FlagEnabled=0`, `FlagDefault=0`) preservation in
   `owned_mp4_video_bridge::prepare_selected`.
3. `fvid::media::probe_as` / `fvid::media_probe::try_probe_as` format hint parity
   with `fvid_media::probe_as` (`"3gp"`, `"3g2"`, `"mj2"`, `"mov,mp4,m4a,3gp,3g2,mj2"`,
   `"matroska,webm"`).
"""

from __future__ import annotations

import hashlib
import json
from pathlib import Path
import struct

from generate_avc_mbaff_direct_samples import Writer
from generate_mp4_audio_selection_regressions import (
    build_aac_packet,
    build_esds,
    make_audio_track,
    mp4a_v0_header,
)
from generate_vp9_chroma import atom, word


ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / "tests/fixtures/playback-errors/rotated-mp4-decode-export-probe"


def build_avc_32x16_ipcm() -> tuple[bytes, list[bytes]]:
    """Build 32x16 (2x1 macroblocks) 4:2:0 8-bit progressive AVC avcC and 2 I_PCM frames."""
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
    b.ue(1)  # pic_width_in_mbs_minus1 = 1 (32 px)
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

        # MB 0 (left 16x16): Y = 40 + frame_idx * 10, Cb = 80, Cr = 160
        # MB 1 (right 16x16): Y = 200 + frame_idx * 10, Cb = 100, Cr = 180
        for mb_x in range(2):
            s.ue(25)  # mb_type = I_PCM (25)
            s.align()
            y_val = (40 if mb_x == 0 else 200) + frame_idx * 10
            cb_val = 80 if mb_x == 0 else 100
            cr_val = 160 if mb_x == 0 else 180
            for _ in range(256):
                s.u(y_val, 8)
            for _ in range(64):
                s.u(cb_val, 8)
            for _ in range(64):
                s.u(cr_val, 8)
        nal = s.nal(0x65 if frame_idx == 0 else 0x41)
        packets.append(len(nal).to_bytes(4, "big") + nal)
    return avcc, packets


def build_rotated_avc_video_track(
    track_id: int,
    avcc: bytes,
    packets: list[bytes],
    offset: int,
    rotation: int = 90,
    pasp: tuple[int, int] | None = (2, 1),
    rate: int = 48000,
    delta: int = 1024,
) -> bytes:
    entry = bytearray(78)
    entry[6:8] = struct.pack(">H", 1)
    entry[24:28] = struct.pack(">HH", 32, 16)
    entry[28:36] = word(72 << 16, 72 << 16)
    entry[40:42] = struct.pack(">H", 1)
    entry[74:78] = struct.pack(">HH", 24, 65535)
    children = atom(b"avcC", avcc)
    if pasp is not None:
        children += atom(b"pasp", word(pasp[0], pasp[1]))
    avc1_box = atom(b"avc1", bytes(entry) + children)
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
    struct.pack_into(">H", mdhd, 20, 0x15C7)  # "eng"
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
    # Matrix at bytes 40..76 (9 big-endian i32s: a, b, u, c, d, v, x, y, w)
    if rotation == 90:
        matrix_vals = [0, 65536, 0, -65536, 0, 0, 0, 0, 1 << 30]
    elif rotation == 180:
        matrix_vals = [-65536, 0, 0, 0, -65536, 0, 0, 0, 1 << 30]
    elif rotation == 270:
        matrix_vals = [0, -65536, 0, 65536, 0, 0, 0, 0, 1 << 30]
    else:
        matrix_vals = [65536, 0, 0, 0, 65536, 0, 0, 0, 1 << 30]
    tkhd[40:76] = b"".join(struct.pack(">i", v) for v in matrix_vals)
    tkhd[76:84] = word(32 << 16, 16 << 16)
    udta = atom(b"udta", atom(b"name", b"RotatedVideo"))
    return atom(b"trak", atom(b"tkhd", bytes(tkhd)) + mdia + udta)


def build_single_rotated_mp4(avcc: bytes, avc_packets: list[bytes]) -> bytes:
    ftyp = atom(b"ftyp", b"isom\x00\x00\x00\x00isomiso6avc1mp41")
    v_payload = b"".join(avc_packets)
    mdat = atom(b"mdat", v_payload)
    v_offset = len(ftyp) + 8
    v_trak = build_rotated_avc_video_track(
        1, avcc, avc_packets, v_offset, rotation=90, pasp=(2, 1), rate=48000, delta=1024
    )
    mvhd = bytearray(100)
    mvhd[12:16] = word(48000)
    mvhd[16:20] = word(2048)
    mvhd[20:24] = word(65536)
    mvhd[24:28] = word(0x01000000)
    for at, val in [(36, 65536), (52, 65536), (68, 1 << 30), (96, 2)]:
        mvhd[at : at + 4] = word(val)
    return ftyp + mdat + atom(b"moov", atom(b"mvhd", bytes(mvhd)) + v_trak)


def build_multitrack_rotated_mp4(avcc: bytes, avc_packets: list[bytes]) -> bytes:
    ftyp = atom(b"ftyp", b"isom\x00\x00\x00\x00isomiso6avc1mp41")
    aac_packets = [build_aac_packet(100), build_aac_packet(100)]
    v_payload = b"".join(avc_packets)
    a_payload = b"".join(aac_packets)
    mdat = atom(b"mdat", v_payload + a_payload + a_payload)

    v_offset = len(ftyp) + 8
    a1_offset = v_offset + len(v_payload)
    a2_offset = a1_offset + len(a_payload)

    asc = b"\x11\x88"
    mp4a_payload = mp4a_v0_header(1, 48000) + atom(b"esds", build_esds(asc))

    v_trak = build_rotated_avc_video_track(
        1, avcc, avc_packets, v_offset, rotation=90, pasp=(2, 1), rate=48000, delta=1024
    )
    a1_trak = make_audio_track(2, 3, b"mp4a", mp4a_payload, aac_packets, a1_offset, 48000, 1024)
    a2_trak = make_audio_track(3, 0, b"mp4a", mp4a_payload, aac_packets, a2_offset, 48000, 1024)

    mvhd = bytearray(100)
    mvhd[12:16] = word(48000)
    mvhd[16:20] = word(2048)
    mvhd[20:24] = word(65536)
    mvhd[24:28] = word(0x01000000)
    for at, val in [(36, 65536), (52, 65536), (68, 1 << 30), (96, 4)]:
        mvhd[at : at + 4] = word(val)
    return ftyp + mdat + atom(b"moov", atom(b"mvhd", bytes(mvhd)) + v_trak + a1_trak + a2_trak)


def generate() -> dict[str, bytes]:
    avcc, avc_packets = build_avc_32x16_ipcm()
    files = {
        "avc-rotated-90-anamorphic.mp4": build_single_rotated_mp4(avcc, avc_packets),
        "avc-rotated-90-multitrack-aac.mp4": build_multitrack_rotated_mp4(avcc, avc_packets),
    }
    manifest = {
        "batch_id": "FVID-0242",
        "generator": "scripts/generate_rotated_mp4_decode_export_probe_regressions.py",
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
