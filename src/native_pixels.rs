//! Owned pointwise operations on packed decoded sample planes.
use crate::{Result, invalid, native_geometry::GeometryFrame};

/// Invert every colour sample. The legacy `1` option includes alpha; native
/// RGB24 and YUV frames have no alpha, so both modes produce the same pixels.
#[derive(Clone, Copy, Debug)]
pub struct Negate;

impl Negate {
    pub fn parse(args: &str) -> Result<Self> {
        match args {
            "" | "0" | "1" => Ok(Self),
            _ => Err(invalid("negate args must be empty, 0, or 1 (alpha)")),
        }
    }

    /// Invert against the sample maximum, including for limited-range YUV.
    /// High-depth planes contain little-endian u16 samples. Validate the whole
    /// buffer before mutation, so malformed input remains unchanged on error.
    pub fn apply(self, frame: &mut GeometryFrame, depth: u8) -> Result<()> {
        if !(8..=16).contains(&depth) || (frame.subsampling.is_none() && depth != 8) {
            return Err(invalid("unsupported negate sample depth"));
        }
        if depth == 8 {
            for sample in &mut frame.data {
                *sample = 255 - *sample;
            }
        } else {
            let max = ((1u32 << depth) - 1) as u16;
            if frame.data.len() % 2 != 0
                || frame
                    .data
                    .chunks_exact(2)
                    .any(|s| u16::from_le_bytes([s[0], s[1]]) > max)
            {
                return Err(invalid("invalid negate sample storage"));
            }
            for sample in frame.data.chunks_exact_mut(2) {
                let value = max - u16::from_le_bytes([sample[0], sample[1]]);
                sample.copy_from_slice(&value.to_le_bytes());
            }
        }
        Ok(())
    }
}
