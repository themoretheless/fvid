//! Owned depth-aware negate for planar sample buffers.
type Result<T> = std::result::Result<T, String>;
fn invalid(message: &str) -> String {
    message.into()
}
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
    pub fn apply(self, data: &mut [u8], depth: u8) -> Result<()> {
        if !(8..=16).contains(&depth) {
            return Err(invalid("unsupported negate sample depth"));
        }
        if depth == 8 {
            for sample in &mut *data {
                *sample = 255 - *sample;
            }
        } else {
            let max = ((1u32 << depth) - 1) as u16;
            if data.len() % 2 != 0
                || data
                    .chunks_exact(2)
                    .any(|s| u16::from_le_bytes([s[0], s[1]]) > max)
            {
                return Err(invalid("invalid negate sample storage"));
            }
            for sample in data.chunks_exact_mut(2) {
                let value = max - u16::from_le_bytes([sample[0], sample[1]]);
                sample.copy_from_slice(&value.to_le_bytes());
            }
        }
        Ok(())
    }
}
