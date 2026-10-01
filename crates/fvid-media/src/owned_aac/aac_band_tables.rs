// Copyright (c) 2019-2026 The Project Symphonia Developers.
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. See LICENSE-MPL-2.0 or https://mozilla.org/MPL/2.0/.
// Numeric AAC band tables extracted from Symphonia 0.6.1 common.rs.
#[rustfmt::skip]
pub(super) const SWB_OFFSET_48K_LONG: [usize; 49 + 1] = [
    0, 4, 8, 12, 16, 20, 24, 28, 32, 36, 40, 48, 56, 64, 72, 80, 88, 96, 108, 120, 132, 144, 160,
    176, 196, 216, 240, 264, 292, 320, 352, 384, 416, 448, 480, 512, 544, 576, 608, 640, 672, 704,
    736, 768, 800, 832, 864, 896, 928, 1024,
];
#[rustfmt::skip]
pub(super) const SWB_OFFSET_48K_SHORT: [usize; 14 + 1] = [0, 4, 8, 12, 16, 20, 28, 36, 44, 56, 68, 80, 96, 112, 128];
#[rustfmt::skip]
pub(super) const SWB_OFFSET_32K_LONG: [usize; 51 + 1] = [
    0, 4, 8, 12, 16, 20, 24, 28, 32, 36, 40, 48, 56, 64, 72, 80, 88, 96, 108, 120, 132, 144, 160,
    176, 196, 216, 240, 264, 292, 320, 352, 384, 416, 448, 480, 512, 544, 576, 608, 640, 672, 704,
    736, 768, 800, 832, 864, 896, 928, 960, 992, 1024,
];
#[rustfmt::skip]
pub(super) const SWB_OFFSET_8K_LONG: [usize; 40 + 1] = [
    0, 12, 24, 36, 48, 60, 72, 84, 96, 108, 120, 132, 144, 156, 172, 188, 204, 220, 236, 252, 268,
    288, 308, 328, 348, 372, 396, 420, 448, 476, 508, 544, 580, 620, 664, 712, 764, 820, 880, 944,
    1024,
];
#[rustfmt::skip]
pub(super) const SWB_OFFSET_8K_SHORT: [usize; 15 + 1] = [0, 4, 8, 12, 16, 20, 24, 28, 36, 44, 52, 60, 72, 88, 108, 128];
#[rustfmt::skip]
pub(super) const SWB_OFFSET_16K_LONG: [usize; 43 + 1] = [
    0, 8, 16, 24, 32, 40, 48, 56, 64, 72, 80, 88, 100, 112, 124, 136, 148, 160, 172, 184, 196, 212,
    228, 244, 260, 280, 300, 320, 344, 368, 396, 424, 456, 492, 532, 572, 616, 664, 716, 772, 832,
    896, 960, 1024,
];
#[rustfmt::skip]
pub(super) const SWB_OFFSET_16K_SHORT: [usize; 15 + 1] = [0, 4, 8, 12, 16, 20, 24, 28, 32, 40, 48, 60, 72, 88, 108, 128];
#[rustfmt::skip]
pub(super) const SWB_OFFSET_24K_LONG: [usize; 47 + 1] = [
    0, 4, 8, 12, 16, 20, 24, 28, 32, 36, 40, 44, 52, 60, 68, 76, 84, 92, 100, 108, 116, 124, 136,
    148, 160, 172, 188, 204, 220, 240, 260, 284, 308, 336, 364, 396, 432, 468, 508, 552, 600, 652,
    704, 768, 832, 896, 960, 1024,
];
#[rustfmt::skip]
pub(super) const SWB_OFFSET_24K_SHORT: [usize; 15 + 1] = [0, 4, 8, 12, 16, 20, 24, 28, 36, 44, 52, 64, 76, 92, 108, 128];
#[rustfmt::skip]
pub(super) const SWB_OFFSET_64K_LONG: [usize; 47 + 1] = [
    0, 4, 8, 12, 16, 20, 24, 28, 32, 36, 40, 44, 48, 52, 56, 64, 72, 80, 88, 100, 112, 124, 140,
    156, 172, 192, 216, 240, 268, 304, 344, 384, 424, 464, 504, 544, 584, 624, 664, 704, 744, 784,
    824, 864, 904, 944, 984, 1024,
];
#[rustfmt::skip]
pub(super) const SWB_OFFSET_64K_SHORT: [usize; 12 + 1] = [0, 4, 8, 12, 16, 20, 24, 32, 40, 48, 64, 92, 128];
#[rustfmt::skip]
pub(super) const SWB_OFFSET_96K_LONG: [usize; 41 + 1] = [
    0, 4, 8, 12, 16, 20, 24, 28, 32, 36, 40, 44, 48, 52, 56, 64, 72, 80, 88, 96, 108, 120, 132,
    144, 156, 172, 188, 212, 240, 276, 320, 384, 448, 512, 576, 640, 704, 768, 832, 896, 960, 1024,
];
// The 960/120 geometry retains lower boundaries and ends at the shorter
// transform size. Derived from the existing normative 1024/128 tables.
const fn shorten<const N: usize>(source: &[usize], end: usize) -> [usize; N] {
    let mut result = [0; N];
    let mut index = 0;
    while index + 1 < N {
        assert!(source[index] < end);
        result[index] = source[index];
        index += 1;
    }
    assert!(source[index] >= end);
    result[index] = end;
    result
}
pub(super) const SWB_960_96K: [usize; 41] = shorten(&SWB_OFFSET_96K_LONG, 960);
pub(super) const SWB_960_64K: [usize; 47] = shorten(&SWB_OFFSET_64K_LONG, 960);
pub(super) const SWB_960_48K: [usize; 50] = shorten(&SWB_OFFSET_48K_LONG, 960);
pub(super) const SWB_960_32K: [usize; 50] = shorten(&SWB_OFFSET_32K_LONG, 960);
pub(super) const SWB_960_24K: [usize; 47] = shorten(&SWB_OFFSET_24K_LONG, 960);
pub(super) const SWB_960_16K: [usize; 43] = shorten(&SWB_OFFSET_16K_LONG, 960);
pub(super) const SWB_960_8K: [usize; 41] = shorten(&SWB_OFFSET_8K_LONG, 960);
pub(super) const SWB_120_64K: [usize; 13] = shorten(&SWB_OFFSET_64K_SHORT, 120);
pub(super) const SWB_120_48K: [usize; 15] = shorten(&SWB_OFFSET_48K_SHORT, 120);
pub(super) const SWB_120_24K: [usize; 16] = shorten(&SWB_OFFSET_24K_SHORT, 120);
pub(super) const SWB_120_16K: [usize; 16] = shorten(&SWB_OFFSET_16K_SHORT, 120);
pub(super) const SWB_120_8K: [usize; 16] = shorten(&SWB_OFFSET_8K_SHORT, 120);
