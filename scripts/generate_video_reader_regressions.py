#!/usr/bin/env python3
"""Purely author playback video reader regression fixtures:
1. Matroska reordered HEVC (Picture::Packed) reorder queue memory budget enforcement.
2. Matroska HEVC with empty hvcC array and in-band VPS/SPS/PPS arriving in packet 1
   after a non-parameter prefix packet 0, exercising bitstream_colour() before SPS arrival.
3. MP4 video reader current_pts() before any frame is read and after rewind().
"""
import hashlib
import json
import struct
from pathlib import Path
from generate_avc_colour_formats import avc_configuration, make_pcm_packets
from generate_ffv1_gray_fixtures import element, uint
from generate_hevc_matroska_formats import D, child, unpack

OUT = Path(__file__).resolve().parents[1] / "tests/fixtures/playback-errors/video-readers"


def generate():
    files = {}

    # 1. Reordered AVC 4:4:4 (Picture::Packed) in Matroska (32x16 8-bit 4:4:4 with reversed PTS across 6 packets)
    # All 6 packets are IDR (nal_unit_type=5, 0x65) so DPB holds at most 1 reference frame while
    # the Matroska reorder queue holds 6 decoded PackedPlanar frames when PTS is reversed.
    avc_cfg = avc_configuration(3, 8, 8, False)
    avc_packets, _ = make_pcm_packets(3, 8, 8)
    avc_packets = [avc_packets[0]] * 6
    avc_track = (
        uint(0xD7, 1)
        + uint(0x83, 1)
        + element(0x86, b"V_MPEG4/ISO/AVC")
        + element(0x63A2, avc_cfg)
        + uint(0x23E383, 40_000_000)
        + element(0xE0, uint(0xB0, 32) + uint(0xBA, 16))
    )
    avc_clusters = b"".join(
        element(
            0x1F43B675,
            uint(0xE7, (5 - idx) * 40) + element(0xA3, b"\x81\x00\x00\x80" + pkt),
        )
        for idx, pkt in enumerate(avc_packets)
    )
    reordered_mkv = element(0x1A45DFA3, element(0x4282, b"matroska")) + element(
        0x18538067,
        element(0x1549A966, uint(0x2AD7B1, 1_000_000))
        + element(0x1654AE6B, element(0xAE, avc_track))
        + avc_clusters,
    )
    files["avc-reordered-packed-budget.mkv"] = reordered_mkv

    # 2. Matroska HEVC with empty hvcC array (22-byte header + numOfArrays=0) where packet 0
    # is an AUD NAL (nal_type=35) and packet 1 carries VPS+SPS+PPS+IDR.
    # At open(), WebmVideoReader observes only packet 0, so decoder.try_parameters() is None when
    # NativeReader::colour() / WebmVideoReader::bitstream_colour() is called right after open().
    rext_path = D / "hevc-pcm-reference-rext8.mp4"
    rext_cfg, rext_packets, rw, rh, _ = unpack(rext_path)
    # Extract VPS/SPS/PPS NALs from rext_cfg
    num_arrays = rext_cfg[22]
    pos = 23
    param_nals = []
    for _ in range(num_arrays):
        _nal_type = rext_cfg[pos] & 0x3F
        num_nals = int.from_bytes(rext_cfg[pos + 1 : pos + 3], "big")
        pos += 3
        for _ in range(num_nals):
            nlen = int.from_bytes(rext_cfg[pos : pos + 2], "big")
            pos += 2
            param_nals.append(rext_cfg[pos : pos + nlen])
            pos += nlen
    empty_hvcc = rext_cfg[:22] + b"\x00"
    aud_nal = bytes([(35 << 1), 1, 0x80])
    pkt0_aud = struct.pack(">I", len(aud_nal)) + aud_nal
    pkt1_inband = b"".join(struct.pack(">I", len(n)) + n for n in param_nals) + rext_packets[0]

    inband_track = (
        uint(0xD7, 1)
        + uint(0x83, 1)
        + element(0x86, b"V_MPEGH/ISO/HEVC")
        + element(0x63A2, empty_hvcc)
        + uint(0x23E383, 40_000_000)
        + element(0xE0, uint(0xB0, rw) + uint(0xBA, rh))
    )
    inband_clusters = element(
        0x1F43B675,
        uint(0xE7, 0)
        + element(0xA3, b"\x81\x00\x00\x00" + pkt0_aud)
        + element(0xA3, b"\x81\x00\x28\x80" + pkt1_inband),
    )
    inband_mkv = element(0x1A45DFA3, element(0x4282, b"matroska")) + element(
        0x18538067,
        element(0x1549A966, uint(0x2AD7B1, 1_000_000))
        + element(0x1654AE6B, element(0xAE, inband_track))
        + inband_clusters,
    )
    files["hevc-inband-deferred-sps.mkv"] = inband_mkv

    # 3. Minimal synthetic MP4 control for NativeReader::current_pts before frame 1 and after rewind
    mp4_bytes = rext_path.read_bytes()
    files["hevc-minimal-pts.mp4"] = mp4_bytes

    manifest = {
        "generator": "scripts/generate_video_reader_regressions.py",
        "fixtures": [
            {
                "file": name,
                "size": len(data),
                "sha256": hashlib.sha256(data).hexdigest(),
            }
            for name, data in sorted(files.items())
        ],
    }
    files["manifest.json"] = (json.dumps(manifest, indent=2) + "\n").encode()
    return files


def main():
    OUT.mkdir(parents=True, exist_ok=True)
    for name, data in generate().items():
        (OUT / name).write_bytes(data)


if __name__ == "__main__":
    main()
