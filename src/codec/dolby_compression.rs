//! Shared owned Dolby compression-gain policy; output calibration is separate.
/// Compression metadata to apply; line mode preserves the default `dynrng` behavior.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CompressionMode {
    #[default]
    Line,
    /// Prefer each frame's `compr`/`compr2`, falling back independently to `dynrng`.
    /// This selects compression gain, not device/RF-modulator level calibration.
    Rf,
}
impl CompressionMode {
    pub(crate) fn gains(self, words: [Option<u8>; 2], line: [f32; 2]) -> [f32; 2] {
        if self == Self::Line {
            return line;
        }
        std::array::from_fn(|i| {
            words[i].map_or(line[i], |word| {
                let exponent = i32::from(word >> 4) - if word & 128 != 0 { 16 } else { 0 };
                2.0f32.powi(exponent) * f32::from(16 + (word & 15)) / 16.0
            })
        })
    }
}
