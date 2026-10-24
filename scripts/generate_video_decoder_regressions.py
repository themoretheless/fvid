#!/usr/bin/env python3
"""Generate synthetic video decoder regression fixtures without FFmpeg or network access."""

from __future__ import annotations

import pathlib

TRANSITION = [
    0, 0, 0, 0, 0, 0, 0, 0, 20, 21, 22, 23, 24, 25, 26, 27, 28, 29, 30, 31, 32, 33, 34, 35, 36, 37,
    37, 38, 39, 40, 41, 42, 43, 44, 45, 46, 47, 48, 49, 50, 51, 52, 53, 54, 55, 56, 56, 57, 58, 59,
    60, 61, 62, 63, 64, 65, 66, 67, 68, 69, 70, 71, 72, 73, 74, 75, 75, 76, 77, 78, 79, 80, 81, 82,
    83, 84, 85, 86, 87, 88, 89, 90, 91, 92, 93, 94, 94, 95, 96, 97, 98, 99, 100, 101, 102, 103,
    104, 105, 106, 107, 108, 109, 110, 111, 112, 113, 114, 114, 115, 116, 117, 118, 119, 120, 121,
    122, 123, 124, 125, 126, 127, 128, 129, 130, 131, 132, 133, 133, 134, 135, 136, 137, 138, 139,
    140, 141, 142, 143, 144, 145, 146, 147, 148, 149, 150, 151, 152, 152, 153, 154, 155, 156, 157,
    158, 159, 160, 161, 162, 163, 164, 165, 166, 167, 168, 169, 170, 171, 171, 172, 173, 174, 175,
    176, 177, 178, 179, 180, 181, 182, 183, 184, 185, 186, 187, 188, 189, 190, 190, 191, 192, 194,
    194, 195, 196, 197, 198, 199, 200, 201, 202, 202, 204, 205, 206, 207, 208, 209, 209, 210, 211,
    212, 213, 215, 215, 216, 217, 218, 219, 220, 220, 222, 223, 224, 225, 226, 227, 227, 229, 229,
    230, 231, 232, 234, 234, 235, 236, 237, 238, 239, 240, 241, 242, 243, 244, 245, 246, 247, 248,
    248, 0, 0, 0, 0, 0, 0, 0,
]


class RangeWriter:
    def __init__(self) -> None:
        self.low = 0
        self.range = 0xFF00
        self.pending: int | None = None
        self.carry_bytes = 0
        self.output = bytearray()

    def _emit(self, byte: int) -> None:
        self.output.append(byte & 0xFF)

    def _normalize(self) -> None:
        if self.low <= 0xFF00 or self.low >= 0x10000:
            carry = 1 if self.low >= 0x10000 else 0
            if self.pending is not None:
                self._emit((self.pending + carry) & 0xFF)
            while self.carry_bytes != 0:
                self._emit(255 if carry == 0 else 0)
                self.carry_bytes -= 1
            self.pending = (self.low >> 8) & 0xFF
        else:
            self.carry_bytes += 1
        self.low = (self.low & 0xFF) << 8
        self.range <<= 8

    def bit(self, state: list[int], idx: int, one: bool) -> None:
        s = state[idx]
        split = (self.range * s) >> 8
        if one:
            self.low += self.range - split
            self.range = split
            state[idx] = TRANSITION[s]
        else:
            self.range -= split
            state[idx] = (256 - TRANSITION[256 - s]) & 0xFF
        if self.range < 256:
            self._normalize()

    def integer(self, state: list[int], value: int, signed: bool) -> None:
        self.bit(state, 0, value == 0)
        if value == 0:
            return
        magnitude = abs(value)
        exponent = magnitude.bit_length() - 1
        for i in range(exponent + 1):
            self.bit(state, 1 + min(i, 9), i < exponent)
        for i in range(exponent - 1, -1, -1):
            self.bit(state, 22 + min(i, 9), ((magnitude >> i) & 1) != 0)
        if signed:
            self.bit(state, 11 + min(exponent, 10), value < 0)

    def finish(self) -> bytes:
        self.low += 255
        self.range = 255
        self._normalize()
        self.range = 255
        self._normalize()
        return bytes(self.output)


def ffv1_crc(data: bytes) -> int:
    crc = 0
    for byte in data:
        crc ^= (byte & 0xFF) << 24
        for _ in range(8):
            if crc & 0x80000000:
                crc = ((crc << 1) ^ 0x04C11DB7) & 0xFFFFFFFF
            else:
                crc = (crc << 1) & 0xFFFFFFFF
    return crc


def append_ffv1_crc(payload: bytes) -> bytes:
    rem = ffv1_crc(payload)
    return payload + rem.to_bytes(4, "big")


def build_ffv1_v3_config() -> bytes:
    """FFV1 v3 configuration with 2 quantizer sets: set 0 has 1 context, set 1 has 2 contexts."""
    w = RangeWriter()
    h = [128] * 32
    w.integer(h, 3, False)       # version = 3
    w.integer(h, 4, False)       # micro_version = 4
    w.integer(h, 1, False)       # coder = 1 (range default transitions)
    w.integer(h, 0, False)       # colorspace = 0 (YCbCr)
    w.integer(h, 8, False)       # bits_per_raw_sample = 8
    w.bit(h, 0, False)           # chroma_planes = false (monochrome)
    w.integer(h, 0, False)       # log2_h_chroma_subsample = 0
    w.integer(h, 0, False)       # log2_v_chroma_subsample = 0
    w.bit(h, 0, False)           # extra_plane (alpha) = false
    w.integer(h, 0, False)       # num_h_slices - 1 = 0 (1x1 slice grid)
    w.integer(h, 0, False)       # num_v_slices - 1 = 0
    w.integer(h, 2, False)       # quant_table_set_count = 2

    # Set 0: all 5 tables have 1 run of 128 (value=1 -> scale=1 -> contexts = 1)
    for _ in range(5):
        m = [128] * 32
        w.integer(m, 127, False)

    # Set 1: table 0 has two runs of 64 (value=2 -> scale=3), tables 1..4 have 1 run of 128 -> contexts = ceil(3/2) = 2
    m0 = [128] * 32
    w.integer(m0, 63, False)
    w.integer(m0, 63, False)
    for _ in range(4):
        m = [128] * 32
        w.integer(m, 127, False)

    # states_coded = false for both sets (use default [128; 32] initial states)
    w.bit(h, 0, False)
    w.bit(h, 0, False)

    w.integer(h, 0, False)       # ec = 0 (3-byte slice footer without CRC)
    w.integer(h, 0, False)       # intra = 0
    return append_ffv1_crc(w.finish())


def build_ffv1_v3_slice_packet(keyframe: bool, quant_set: int, residuals: list[int]) -> bytes:
    """Encode a 1-slice FFV1 v3 monochrome 2x2 packet with the given quantizer set and residuals."""
    w = RangeWriter()
    kstate = [128]
    w.bit(kstate, 0, keyframe)
    h = [128] * 32
    w.integer(h, 0, False)       # slice_x = 0
    w.integer(h, 0, False)       # slice_y = 0
    w.integer(h, 0, False)       # slice_width - 1 = 0
    w.integer(h, 0, False)       # slice_height - 1 = 0
    w.integer(h, quant_set, False)  # quant_table_set_index[0]
    w.integer(h, 0, False)       # quant_table_set_index[1]
    w.integer(h, 0, False)       # picture_structure = 0
    w.integer(h, 0, False)       # sar_num = 0
    w.integer(h, 0, False)       # sar_den = 0

    # All 4 pixels in our 2x2 frame have zero neighbors when previous pixels are 0 or first pixel is 0,
    # Wait: if pixel 0 has residual 0, pixel 0 is 0; if pixel 3 (bottom-right) has residual 0, etc.
    # Even simpler: if pixel 0 has residual 0, pixel 1 has residual 0, pixel 2 has residual 0,
    # and pixel 3 (bottom-right, where l=0, t=0, tl=0 -> context=0) has residual = residuals[3]!
    # That way EVERY pixel uses context 0 regardless of quantizer table, and we don't even have to track context switching!
    model0 = [128] * 32
    for r in residuals:
        w.integer(model0, r, True)
    payload = w.finish()
    footer = len(payload).to_bytes(3, "big")
    return payload + footer


class BitWriter:
    def __init__(self) -> None:
        self.bits: list[int] = []

    def write(self, value: int, width: int) -> None:
        for i in range(width - 1, -1, -1):
            self.bits.append((value >> i) & 1)

    def to_bytes(self) -> bytes:
        out = bytearray((len(self.bits) + 7) // 8)
        for i, bit in enumerate(self.bits):
            if bit:
                out[i // 8] |= 1 << (7 - (i % 8))
        return bytes(out)


def build_av1_grain_cb_nonzero_cr_zero() -> bytes:
    """AV1 4:2:0 keyframe header payload with film_grain_params where num_cb_points=1 and num_cr_points=0."""
    bw = BitWriter()
    bw.write(0, 1)        # show_existing_frame = 0
    bw.write(0, 2)        # frame_type = 0 (KEY_FRAME)
    bw.write(1, 1)        # show_frame = 1 (error_resilient = 1, refresh_flags = 255)
    bw.write(1, 1)        # disable_cdf_update = 1 (disable_frame_end_update_cdf inferred = 1)
    bw.write(0, 1)        # allow_screen_content_tools = 0 (sequence has screen_content_tools = 2)
    bw.write(0, 1)        # frame_size_override_flag = 0 (size = 64x64)
    bw.write(0, 7)        # order_hint (order_hint_bits = 7 in sequence.obu)
    bw.write(0, 1)        # render_and_frame_size_different = 0
    # Tile info for 64x64 with 64x64 superblocks (1x1 sb):
    bw.write(1, 1)        # uniform_tile_spacing_flag = 1 (max_log2_tile_cols=0, max_log2_tile_rows=0 -> 0 extra bits)
    # Quantization params:
    bw.write(0, 8)        # base_q_idx = 0
    bw.write(0, 1)        # delta_q_y_dc = 0
    bw.write(0, 1)        # delta_q_u_dc = 0
    bw.write(0, 1)        # delta_q_u_ac = 0
    bw.write(0, 1)        # using_qmatrix = 0
    # Segmentation:
    bw.write(0, 1)        # segmentation_enabled = 0
    # base_q_idx == 0 and deltas == 0 -> coded_lossless = true, all_lossless = true!
    # Therefore loop_filter, cdef, restoration, and tx_mode are all skipped!
    bw.write(0, 1)        # reduced_tx_set = 0
    # Now film_grain_params()!
    bw.write(1, 1)        # apply_grain = 1
    bw.write(1234, 16)    # grain_seed = 1234
    bw.write(1, 4)        # num_y_points = 1
    bw.write(16, 8)       # point_y_value[0]
    bw.write(32, 8)       # point_y_scaling[0]
    bw.write(0, 1)        # chroma_scaling_from_luma = 0
    bw.write(1, 4)        # num_cb_points = 1 (nonzero!)
    bw.write(16, 8)       # point_cb_value[0]
    bw.write(32, 8)       # point_cb_scaling[0]
    bw.write(0, 4)        # num_cr_points = 0 (zero! Forbidden by AV1 6.8.20 when 4:2:0 and num_cb_points != 0)
    bw.write(0, 2)        # grain_scaling_minus_8
    bw.write(0, 2)        # ar_coeff_lag = 0
    bw.write(0, 8)        # ar_coeffs_cb_plus_128[0] (since num_y_points != 0 and num_cb_points != 0)
    bw.write(0, 2)        # ar_coeff_shift_minus_6
    bw.write(0, 2)        # grain_scale_shift
    bw.write(128, 8)      # cb_mult
    bw.write(128, 8)      # cb_luma_mult
    bw.write(256, 9)      # cb_offset
    bw.write(1, 1)        # overlap_flag
    bw.write(0, 1)        # clip_to_restricted_range
    # byte alignment for OBU_FRAME header:
    while len(bw.bits) % 8 != 0:
        bw.write(0, 1)
    return bw.to_bytes() + b"\x00"


def generate(dest: pathlib.Path) -> dict[str, bytes]:
    files = {
        "ffv1_v3_two_quant_sets.cfg": build_ffv1_v3_config(),
        "ffv1_v3_keyframe_set0.pkt": build_ffv1_v3_slice_packet(True, 0, [0, 0, 0, 10]),
        "ffv1_v3_inter_grow_set1.pkt": build_ffv1_v3_slice_packet(False, 1, [0, 0, 0, 42]),
        "av1_grain_420_cb1_cr0.bin": build_av1_grain_cb_nonzero_cr_zero(),
    }
    dest.mkdir(parents=True, exist_ok=True)
    for name, content in files.items():
        (dest / name).write_bytes(content)
    return files


if __name__ == "__main__":
    root = pathlib.Path(__file__).resolve().parents[1]
    out_dir = root / "tests" / "fixtures" / "playback-errors" / "video-decoders"
    generated = generate(out_dir)
    for k, v in sorted(generated.items()):
        print(f"{k}: {len(v)} bytes")
