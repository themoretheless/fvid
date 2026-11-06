#!/usr/bin/env python3
"""Offline generator for Batch 572 (`FVID-0590..0594`) synthetic media fixtures.

Writes deterministic synthetic reproducers into:
  tests/fixtures/playback-errors/y4m-adts-and-mkv-probe/

1. `aac-main-48k-mono.aac` (FVID-0590):
   2-frame bare ADTS AAC Main (`profile_ObjectType = 0` -> AudioObjectType 1 "Main")
   stream at 48 kHz mono (1024 samples/frame = 2048 samples total).
2. `aac-lc-pce-48k-stereo.aac` (FVID-0590):
   1-frame bare ADTS AAC-LC stream with `channel_configuration == 0` and an in-band
   Program Config Element (PCE) declaring stereo (`extradata_bytes > 2`).
3. `omitted-rate-2x2.y4m` (FVID-0591):
   Y4M 2x2 stream without an explicit `F` frame-rate token (defaults to 25:1 per Y4M spec).
4. `c440-8bit-2x4.y4m` and `c411-8bit-4x2.y4m` and `c410-8bit-4x4.y4m` (FVID-0592):
   8-bit Y4M streams using `C440`, `C411`, and `C410` planar chroma subsampling.
5. `c411p10-4x2.y4m` and `c410p12-4x4.y4m` (FVID-0593):
   High-bit-depth 10-bit `C411p10` and 12-bit `C410p12` Y4M streams.
6. `chapters-open-end.mkv` (FVID-0594):
   Matroska container with two `ChapterAtom` entries that omit `ChapterTimeEnd`,
   verifying that `fvid_media::owned_webm_probe` infers open-ended chapter ends
   from the next chapter start and container duration.
"""

from pathlib import Path


class BitWriter:
    def __init__(self) -> None:
        self.bits: list[int] = []

    def write(self, value: int, width: int) -> None:
        for shift in range(width - 1, -1, -1):
            self.bits.append((value >> shift) & 1)

    def byte_align(self) -> None:
        while len(self.bits) % 8 != 0:
            self.bits.append(0)

    def to_bytes(self) -> bytes:
        self.byte_align()
        out = bytearray()
        for i in range(0, len(self.bits), 8):
            byte = 0
            for b in self.bits[i : i + 8]:
                byte = (byte << 1) | b
            out.append(byte)
        return bytes(out)


def adts_frame(profile_object_type: int, freq_index: int, chan_config: int, raw: bytes) -> bytes:
    frame_len = 7 + len(raw)
    b0 = 0xFF
    b1 = 0xF1  # MPEG-4, layer 0, protection_absent=1
    b2 = ((profile_object_type & 0x3) << 6) | ((freq_index & 0xF) << 2) | ((chan_config >> 2) & 0x1)
    b3 = ((chan_config & 0x3) << 6) | ((frame_len >> 11) & 0x3)
    b4 = (frame_len >> 3) & 0xFF
    b5 = ((frame_len & 0x7) << 5) | 0x1F
    b6 = 0xFC  # fullness all 1s, raw_data_blocks_in_frame = 0
    return bytes([b0, b1, b2, b3, b4, b5, b6]) + raw


def build_aac_main_adts() -> bytes:
    # Minimal raw block with END (id_syn_ele = 7)
    bw = BitWriter()
    bw.write(7, 3)  # ID_END
    raw = bw.to_bytes()
    # profile_ObjectType = 0 -> AudioObjectType = 1 ("Main"), freq_index = 3 (48000 Hz), chan_config = 1
    frame = adts_frame(0, 3, 1, raw)
    return frame * 2


def build_aac_lc_pce_adts() -> bytes:
    bw = BitWriter()
    # ID_PCE = 5
    bw.write(5, 3)
    bw.write(0, 4)  # element_instance_tag = 0
    bw.write(1, 2)  # object_type = 1 (AAC-LC in PCE is 1, which maps to AOT 2!)
    bw.write(3, 4)  # sampling_frequency_index = 3 (48000 Hz)
    bw.write(1, 4)  # num_front_channel_elements = 1
    bw.write(0, 4)  # num_side_channel_elements = 0
    bw.write(0, 4)  # num_back_channel_elements = 0
    bw.write(0, 2)  # num_lfe_channel_elements = 0
    bw.write(0, 3)  # num_assoc_data_elements = 0
    bw.write(0, 4)  # num_valid_cc_elements = 0
    bw.write(0, 1)  # mono_mixdown_present = 0
    bw.write(0, 1)  # stereo_mixdown_present = 0
    bw.write(0, 1)  # matrix_mixdown_idx_present = 0
    # front_element[0]: is_cpe = 1 (stereo pair), tag_select = 0
    bw.write(1, 1)
    bw.write(0, 4)
    bw.byte_align()
    bw.write(0, 8)  # comment_field_bytes = 0
    bw.write(7, 3)  # ID_END
    raw = bw.to_bytes()
    # profile_ObjectType = 1 -> AudioObjectType = 2 ("LC"), freq_index = 3 (48000 Hz), chan_config = 0 (PCE)
    return adts_frame(1, 3, 0, raw)


def ebml_vint_size(size: int) -> bytes:
    for width in range(1, 9):
        limit = (1 << (7 * width)) - 1
        if size < limit:
            val = size | (1 << (7 * width))
            return val.to_bytes(width, "big")
    raise ValueError("size too large")


def ebml_el(el_id: bytes, payload: bytes) -> bytes:
    return el_id + ebml_vint_size(len(payload)) + payload


def ebml_uint(el_id: bytes, val: int) -> bytes:
    if val == 0:
        b = b"\x00"
    else:
        length = (val.bit_length() + 7) // 8
        b = val.to_bytes(length, "big")
    return ebml_el(el_id, b)


def ebml_float64(el_id: bytes, val: float) -> bytes:
    import struct

    return ebml_el(el_id, struct.pack(">d", val))


def ebml_str(el_id: bytes, s: str) -> bytes:
    return ebml_el(el_id, s.encode("utf-8"))


def build_chapters_open_end_mkv() -> bytes:
    ebml_head = ebml_el(
        b"\x1a\x45\xdf\xa3",
        ebml_uint(b"\x42\x86", 1)
        + ebml_uint(b"\x42\xf7", 1)
        + ebml_uint(b"\x42\xf2", 4)
        + ebml_uint(b"\x42\xf3", 8)
        + ebml_str(b"\x42\x82", "matroska")
        + ebml_uint(b"\x42\x87", 4)
        + ebml_uint(b"\x42\x85", 2),
    )
    # Info: TimecodeScale = 1_000_000 (1ms), Duration = 100.0 ms (100_000_000 ns)
    info = ebml_el(
        b"\x15\x49\xa9\x66",
        ebml_uint(b"\x2a\xd7\xb1", 1_000_000) + ebml_float64(b"\x44\x89", 100.0),
    )
    # TrackEntry: 16x8 A_PCM/INT/LIT audio or V_FFV1 video
    # Use a simple 8-bit mono PCM audio track so no codec private is needed
    audio = ebml_el(
        b"\xe1",
        ebml_float64(b"\xb5", 8000.0)
        + ebml_uint(b"\x9f", 1)
        + ebml_uint(b"\x62\x64", 8),
    )
    track_entry = ebml_el(
        b"\xae",
        ebml_uint(b"\xd7", 1)
        + ebml_uint(b"\x73\xc5", 1)
        + ebml_uint(b"\x83", 2)
        + ebml_str(b"\x86", "A_PCM/INT/LIT")
        + audio,
    )
    tracks = ebml_el(b"\x16\x54\xae\x6b", track_entry)
    # Chapters: two ChapterAtom entries WITHOUT ChapterTimeEnd (0x92)
    chap1_disp = ebml_el(b"\x80", ebml_str(b"\x85", "Intro"))
    chap1 = ebml_el(
        b"\xb6",
        ebml_uint(b"\x73\xc4", 1)
        + ebml_uint(b"\x91", 10_000_000)
        + chap1_disp,
    )
    chap2_disp = ebml_el(b"\x80", ebml_str(b"\x85", "Outro"))
    chap2 = ebml_el(
        b"\xb6",
        ebml_uint(b"\x73\xc4", 2)
        + ebml_uint(b"\x91", 40_000_000)
        + chap2_disp,
    )
    edition = ebml_el(b"\x45\xb9", chap1 + chap2)
    chapters = ebml_el(b"\x10\x43\xa7\x70", edition)
    # One Cluster at timecode 0 with one SimpleBlock (track 1, timecode 0, keyframe)
    simple_block = ebml_el(b"\xa3", b"\x81\x00\x00\x80" + b"\x80" * 80)
    cluster = ebml_el(b"\x1f\x43\xb6\x75", ebml_uint(b"\xe7", 0) + simple_block)
    segment = ebml_el(b"\x18\x53\x80\x67", info + tracks + chapters + cluster)
    return ebml_head + segment


def main() -> None:
    root = Path(__file__).resolve().parent.parent
    out_dir = root / "tests" / "fixtures" / "playback-errors" / "y4m-adts-and-mkv-probe"
    out_dir.mkdir(parents=True, exist_ok=True)

    (out_dir / "aac-main-48k-mono.aac").write_bytes(build_aac_main_adts())
    (out_dir / "aac-lc-pce-48k-stereo.aac").write_bytes(build_aac_lc_pce_adts())

    # Omitted-rate Y4M (W2 H2 C420jpeg, 1 frame: 4 Y + 1 Cb + 1 Cr)
    (out_dir / "omitted-rate-2x2.y4m").write_bytes(
        b"YUV4MPEG2 W2 H2 Ip C420jpeg\nFRAME\n" + bytes([16, 235, 16, 235, 128, 128])
    )

    # 8-bit C440 (2x4: Y=8, Cb=2x2=4, Cr=4), C411 (4x2: Y=8, Cb=1x2=2, Cr=2), C410 (4x4: Y=16, Cb=1x1=1, Cr=1)
    (out_dir / "c440-8bit-2x4.y4m").write_bytes(
        b"YUV4MPEG2 W2 H4 F25:1 Ip C440\nFRAME\n"
        + bytes([64] * 8 + [128] * 4 + [128] * 4)
    )
    (out_dir / "c411-8bit-4x2.y4m").write_bytes(
        b"YUV4MPEG2 W4 H2 F25:1 Ip C411\nFRAME\n"
        + bytes([80] * 8 + [128] * 2 + [128] * 2)
    )
    (out_dir / "c410-8bit-4x4.y4m").write_bytes(
        b"YUV4MPEG2 W4 H4 F25:1 Ip C410\nFRAME\n"
        + bytes([96] * 16 + [128] + [128])
    )

    # High-bit-depth C411p10 (4x2: Y=8 words, Cb=2 words, Cr=2 words) and C410p12 (4x4: Y=16 words, Cb=1 word, Cr=1 word)
    c411p10_words = [512] * 8 + [512] * 2 + [512] * 2
    (out_dir / "c411p10-4x2.y4m").write_bytes(
        b"YUV4MPEG2 W4 H2 F25:1 Ip C411p10\nFRAME\n"
        + b"".join(w.to_bytes(2, "little") for w in c411p10_words)
    )
    c410p12_words = [2048] * 16 + [2048] + [2048]
    (out_dir / "c410p12-4x4.y4m").write_bytes(
        b"YUV4MPEG2 W4 H4 F25:1 Ip C410p12\nFRAME\n"
        + b"".join(w.to_bytes(2, "little") for w in c410p12_words)
    )

    (out_dir / "chapters-open-end.mkv").write_bytes(build_chapters_open_end_mkv())


if __name__ == "__main__":
    main()
