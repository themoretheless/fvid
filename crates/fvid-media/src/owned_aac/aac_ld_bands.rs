//! AAC-LD long-window geometry, ISO/IEC 14496-3 tables 4.86–4.91.
use super::{Result, invalid};

const B480_48: &[usize] = &[
    0, 4, 8, 12, 16, 20, 24, 28, 32, 36, 40, 44, 48, 52, 56, 64, 72, 80, 88, 96, 108, 120, 132,
    144, 156, 172, 188, 212, 240, 272, 304, 336, 368, 400, 432, 480,
];
const B512_48: &[usize] = &[
    0, 4, 8, 12, 16, 20, 24, 28, 32, 36, 40, 44, 48, 52, 56, 60, 68, 76, 84, 92, 100, 112, 124,
    136, 148, 164, 184, 208, 236, 268, 300, 332, 364, 396, 428, 460, 512,
];
const B480_32: &[usize] = &[
    0, 4, 8, 12, 16, 20, 24, 28, 32, 36, 40, 44, 48, 52, 56, 60, 64, 72, 80, 88, 96, 104, 112, 124,
    136, 148, 164, 180, 200, 224, 256, 288, 320, 352, 384, 416, 448, 480,
];
const B512_32: &[usize] = &[
    0, 4, 8, 12, 16, 20, 24, 28, 32, 36, 40, 44, 48, 52, 56, 64, 72, 80, 88, 96, 108, 120, 132,
    144, 160, 176, 192, 212, 236, 260, 288, 320, 352, 384, 416, 448, 480, 512,
];
const B480_24: &[usize] = &[
    0, 4, 8, 12, 16, 20, 24, 28, 32, 36, 40, 44, 52, 60, 68, 80, 92, 104, 120, 140, 164, 192, 224,
    256, 288, 320, 352, 384, 416, 448, 480,
];
const B512_24: &[usize] = &[
    0, 4, 8, 12, 16, 20, 24, 28, 32, 36, 40, 44, 52, 60, 68, 80, 92, 104, 120, 140, 164, 192, 224,
    256, 288, 320, 352, 384, 416, 448, 480, 512,
];

#[derive(Clone, Copy, Debug)]
pub struct LdBands {
    pub offsets: &'static [usize],
    pub tns_max_bands: usize,
}
impl LdBands {
    pub fn new(sample_rate: u32, frame_samples: usize) -> Result<Self> {
        // LD has no short windows. Do not invent tables for rates outside
        // the standardized LD range; explicit rates use the ordinary intervals.
        let offsets = match (frame_samples, sample_rate) {
            (480, 37566..=55425) => B480_48,
            (512, 37566..=55425) => B512_48,
            (480, 27713..=37565) => B480_32,
            (512, 27713..=37565) => B512_32,
            (480, 18783..=27712) => B480_24,
            (512, 18783..=27712) => B512_24,
            _ => return Err(invalid("unsupported AAC LD band geometry")),
        };
        let tns_max_bands = match sample_rate {
            46009..=55425 => 31,
            37566..=46008 => 32,
            27713..=37565 => 37,
            _ => {
                if frame_samples == 480 {
                    30
                } else {
                    31
                }
            }
        };
        Ok(Self {
            offsets,
            tns_max_bands,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ld_tables_cover_exact_frame_and_standard_band_counts() {
        for (n, rate, count, tns) in [
            (480, 48000, 35, 31),
            (512, 48000, 36, 31),
            (480, 44100, 35, 32),
            (512, 44100, 36, 32),
            (480, 32000, 37, 37),
            (512, 32000, 37, 37),
            (480, 24000, 30, 30),
            (512, 24000, 31, 31),
            (480, 22050, 30, 30),
            (512, 22050, 31, 31),
        ] {
            let b = LdBands::new(rate, n).unwrap();
            assert_eq!(b.offsets.len(), count + 1);
            assert_eq!(b.tns_max_bands, tns);
            assert_eq!(b.offsets[0], 0);
            assert_eq!(b.offsets[count], n);
            assert!(
                b.offsets
                    .windows(2)
                    .all(|w| w[0] < w[1] && (w[1] - w[0]) % 4 == 0)
            );
        }
        for rate in [0, 8000, 16000, 18782, 55426, 96000] {
            assert!(LdBands::new(rate, 512).is_err());
        }
        assert!(LdBands::new(24000, 1024).is_err());
    }
}
