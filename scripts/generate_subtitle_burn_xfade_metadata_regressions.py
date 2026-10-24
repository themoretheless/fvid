#!/usr/bin/env python3
"""Generate deterministic synthetic fixtures for Batch 236 (`FVID-0243`).

Covers:
1. Single-track MP4 video subtitle burn-in (`avc-single-rotated-burn.mp4` + `cues.srt`):
   - 64x32 coded `avc1` I_PCM with `tkhd` rotation 90 (display 32x64) and 2 frames
     (each 500 ms: `rate=2, delta=1`, so frame 0 at `0 ms` and frame 1 at `500 ms`).
   - `cues.srt` with cue `00:00:00,000 --> 00:00:00,400` ("FVid"), active on frame 0
     and inactive on frame 1.
2. Set-then-delete container & stream metadata precedence across:
   - `owned_subtitle_burn::burn_subtitles` (Y4M, Matroska, and MP4 inputs)
   - `owned_xfade::xfade_video` (`xfade-main.y4m` and `xfade-other.y4m`)
   - `owned_subtitle::try_convert_with_controls` (`cues.srt` -> `.mkv`)
   - `owned_lossless::transcode_lossless` (`xfade-main.y4m` -> `.mkv`)
3. RGB and monochrome (`V_FFV1` gray) Matroska subtitle burn-in in
   `owned_text_raster::composite_white` and `owned_ffv1_export::export_processed`.
"""

from __future__ import annotations

import hashlib
import json
from pathlib import Path
import struct

from generate_avc_mbaff_direct_samples import Writer
from generate_vp9_chroma import atom, word


ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / "tests/fixtures/playback-errors/subtitle-burn-xfade-metadata"


def build_avc_64x32_ipcm() -> tuple[bytes, list[bytes]]:
    """Build 64x32 (4x2 macroblocks) 4:2:0 8-bit progressive AVC avcC and 2 I_PCM frames."""
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
    b.ue(3)  # pic_width_in_mbs_minus1 = 3 (64 px)
    b.ue(1)  # pic_height_in_map_units_minus1 = 1 (32 px)
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

        # 8 macroblocks (4x2): uniform dark luma Y=32, Cb=64, Cr=192
        for _ in range(8):
            s.ue(25)  # mb_type = I_PCM (25)
            s.align()
            for _ in range(256):
                s.u(32, 8)
            for _ in range(64):
                s.u(64, 8)
            for _ in range(64):
                s.u(192, 8)
        nal = s.nal(0x65 if frame_idx == 0 else 0x41)
        packets.append(len(nal).to_bytes(4, "big") + nal)
    return avcc, packets


def build_rotated_mp4(avcc: bytes, packets: list[bytes]) -> bytes:
    ftyp = atom(b"ftyp", b"isom\x00\x00\x00\x00isomiso6avc1mp41")
    v_payload = b"".join(packets)
    mdat = atom(b"mdat", v_payload)
    offset = len(ftyp) + 8

    entry = bytearray(78)
    entry[6:8] = struct.pack(">H", 1)
    entry[24:28] = struct.pack(">HH", 64, 32)
    entry[28:36] = word(72 << 16, 72 << 16)
    entry[40:42] = struct.pack(">H", 1)
    entry[74:78] = struct.pack(">HH", 24, 65535)
    avc1_box = atom(b"avc1", bytes(entry) + atom(b"avcC", avcc))
    stbl = (
        atom(b"stsd", word(0, 1) + avc1_box)
        + atom(b"stts", word(0, 1, len(packets), 1))
        + atom(b"stsc", word(0, 1, 1, len(packets), 1))
        + atom(b"stsz", word(0, 0, len(packets), *map(len, packets)))
        + atom(b"stco", word(0, 1, offset))
        + atom(b"stss", word(0, 2, 1, 2))
    )
    dinf = atom(b"dinf", atom(b"dref", word(0, 1) + atom(b"url ", word(1))))
    vmhd = atom(b"vmhd", word(1, 0, 0))
    mdhd = bytearray(24)
    mdhd[12:20] = word(2, len(packets))  # rate=2 Hz, duration=2 (each frame is 500 ms)
    struct.pack_into(">H", mdhd, 20, 0x15C7)  # "eng"
    mdia = atom(
        b"mdia",
        atom(b"mdhd", bytes(mdhd))
        + atom(b"hdlr", word(0, 0) + b"vide" + bytes(12) + b"VideoHandler\x00")
        + atom(b"minf", vmhd + dinf + atom(b"stbl", stbl)),
    )
    tkhd = bytearray(84)
    tkhd[:4] = word(3)
    tkhd[12:16] = word(1)
    tkhd[20:24] = word(len(packets))
    matrix_vals = [0, 65536, 0, -65536, 0, 0, 0, 0, 1 << 30]  # 90 deg clockwise
    tkhd[40:76] = b"".join(struct.pack(">i", v) for v in matrix_vals)
    tkhd[76:84] = word(64 << 16, 32 << 16)
    udta = atom(b"udta", atom(b"name", b"SourceTitle"))
    trak = atom(b"trak", atom(b"tkhd", bytes(tkhd)) + mdia + udta)

    mvhd = bytearray(100)
    mvhd[12:16] = word(2)
    mvhd[16:20] = word(len(packets))
    mvhd[20:24] = word(65536)
    mvhd[24:28] = word(0x01000000)
    for at, val in [(36, 65536), (52, 65536), (68, 1 << 30), (96, 2)]:
        mvhd[at : at + 4] = word(val)
    return ftyp + mdat + atom(b"moov", atom(b"mvhd", bytes(mvhd)) + trak)


def build_y4m(width: int, height: int, luma: int, cb: int, cr: int) -> bytes:
    header = f"YUV4MPEG2 W{width} H{height} F2:1 Ip A1:1 C420\n".encode("ascii")
    frame = (
        b"FRAME\n"
        + bytes([luma] * (width * height))
        + bytes([cb] * ((width // 2) * (height // 2)))
        + bytes([cr] * ((width // 2) * (height // 2)))
    )
    return header + frame + frame


def generate() -> dict[str, bytes]:
    avcc, packets = build_avc_64x32_ipcm()
    srt = (
        "1\n"
        "00:00:00,000 --> 00:00:00,400\n"
        "FVid\n"
    ).encode("utf-8")
    files = {
        "avc-single-rotated-burn.mp4": build_rotated_mp4(avcc, packets),
        "cues.srt": srt,
        "burn-source.y4m": build_y4m(64, 32, 32, 64, 192),
        "xfade-main.y4m": build_y4m(4, 4, 20, 128, 128),
        "xfade-other.y4m": build_y4m(4, 4, 220, 128, 128),
    }
    manifest = {
        "batch_id": "FVID-0243",
        "generator": "scripts/generate_subtitle_burn_xfade_metadata_regressions.py",
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
