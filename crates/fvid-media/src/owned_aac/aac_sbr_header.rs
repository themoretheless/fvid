//! Owned legacy SBR header syntax (MPEG-4 Audio, sbr_header).
use super::{Result, bits::BitReader, invalid};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Header {
    pub amplitude_resolution: bool,
    pub start_frequency: u8,
    pub stop_frequency: u8,
    pub crossover: u8,
    pub frequency_scale: u8,
    pub alter_scale: bool,
    pub noise_bands: u8,
    pub limiter_bands: u8,
    pub limiter_gains: u8,
    pub interpolate_frequency: bool,
    pub smoothing_mode: bool,
}

impl Header {
    /// Whether frequency-dependent decoder history must be reset (6.18.3.1).
    /// A first header initializes the geometry. Amplitude, limiter and smoothing
    /// changes alone preserve history; an output-rate change is handled by the
    /// stream owner separately.
    pub fn requires_reset(&self, previous: Option<&Self>) -> bool {
        let Some(previous) = previous else {
            return true;
        };
        self.start_frequency != previous.start_frequency
            || self.stop_frequency != previous.stop_frequency
            || self.frequency_scale != previous.frequency_scale
            || self.alter_scale != previous.alter_scale
            || self.crossover != previous.crossover
            || self.noise_bands != previous.noise_bands
    }

    /// Read inside an absolute payload bit boundary, committing the reader only
    /// on success. Absent extra fields always select defaults, not old values.
    pub fn read(bits: &mut BitReader<'_>, payload_end: usize) -> Result<Self> {
        let start = bits.position();
        if payload_end < start || payload_end - start > bits.remaining() {
            return Err(invalid("invalid SBR header payload boundary"));
        }
        let mut trial = bits.clone();
        let mut field = |width: u8| -> Result<u8> {
            if usize::from(width) > payload_end.saturating_sub(trial.position()) {
                return Err(invalid("truncated SBR header"));
            }
            Ok(trial.read(width)? as u8)
        };
        let amplitude_resolution = field(1)? != 0;
        let start_frequency = field(4)?;
        let stop_frequency = field(4)?;
        let crossover = field(3)?;
        let _reserved = field(2)?;
        let extra1 = field(1)? != 0;
        let extra2 = field(1)? != 0;
        let (frequency_scale, alter_scale, noise_bands) = if extra1 {
            (field(2)?, field(1)? != 0, field(2)?)
        } else {
            (2, true, 2)
        };
        let (limiter_bands, limiter_gains, interpolate_frequency, smoothing_mode) = if extra2 {
            (field(2)?, field(2)?, field(1)? != 0, field(1)? != 0)
        } else {
            (2, 2, true, true)
        };
        let result = Self {
            amplitude_resolution,
            start_frequency,
            stop_frequency,
            crossover,
            frequency_scale,
            alter_scale,
            noise_bands,
            limiter_bands,
            limiter_gains,
            interpolate_frequency,
            smoothing_mode,
        };
        *bits = trial;
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reset_tracks_exactly_the_six_geometry_fields() {
        let (bytes, end) = sample(0, false, false);
        let original = Header::read(&mut BitReader::new(&bytes), end).unwrap();
        assert!(original.requires_reset(None));
        assert!(!original.requires_reset(Some(&original)));
        // Enumerate every subset of changed fields, including combinations of
        // geometry and non-geometry fields. This is header protocol coverage,
        // not an encoded HE-AAC playback acceptance test.
        for mask in 0..1u16 << 11 {
            let mut next = original.clone();
            if mask & 1 != 0 {
                next.start_frequency ^= 1;
            }
            if mask & 2 != 0 {
                next.stop_frequency ^= 1;
            }
            if mask & 4 != 0 {
                next.frequency_scale ^= 1;
            }
            if mask & 8 != 0 {
                next.alter_scale = !next.alter_scale;
            }
            if mask & 16 != 0 {
                next.crossover ^= 1;
            }
            if mask & 32 != 0 {
                next.noise_bands ^= 1;
            }
            if mask & 64 != 0 {
                next.amplitude_resolution = !next.amplitude_resolution;
            }
            if mask & 128 != 0 {
                next.limiter_bands ^= 1;
            }
            if mask & 256 != 0 {
                next.limiter_gains ^= 1;
            }
            if mask & 512 != 0 {
                next.interpolate_frequency = !next.interpolate_frequency;
            }
            if mask & 1024 != 0 {
                next.smoothing_mode = !next.smoothing_mode;
            }
            assert_eq!(
                next.requires_reset(Some(&original)),
                mask & 63 != 0,
                "mask {mask}"
            );
            assert_eq!(original.requires_reset(Some(&next)), mask & 63 != 0);
        }
    }

    // Hand-authored fields exercise the published syntax, independently of the
    // parser. Padding remains available so only the payload boundary cuts reads.
    fn sample(offset: usize, extra1: bool, extra2: bool) -> (Vec<u8>, usize) {
        let mut text = "1".repeat(offset);
        text.push_str("10101101011100"); // amp=1, start=5, stop=10, xover=7, reserved=0
        text.push(if extra1 { '1' } else { '0' });
        text.push(if extra2 { '1' } else { '0' });
        if extra1 {
            text.push_str("01011");
        } // scale=1, alter=0, noise=3
        if extra2 {
            text.push_str("110100");
        } // bands=3, gains=1, interpolation/smoothing=0
        let end = text.len();
        text.push_str("1111111111111111");
        let mut bytes = vec![0; text.len().div_ceil(8)];
        for (i, bit) in text.bytes().enumerate() {
            if bit == b'1' {
                bytes[i / 8] |= 1 << (7 - i % 8);
            }
        }
        (bytes, end)
    }

    #[test]
    fn syntax_defaults_and_every_truncation_are_transactional() {
        for offset in 0..8 {
            for extra1 in [false, true] {
                for extra2 in [false, true] {
                    let (bytes, end) = sample(offset, extra1, extra2);
                    let mut bits = BitReader::new(&bytes);
                    bits.skip(offset).unwrap();
                    for limit in offset..end {
                        assert!(Header::read(&mut bits, limit).is_err());
                        assert_eq!(bits.position(), offset);
                    }
                    let h = Header::read(&mut bits, end).unwrap();
                    assert_eq!(bits.position(), end);
                    assert!(h.amplitude_resolution);
                    assert_eq!(
                        (h.start_frequency, h.stop_frequency, h.crossover),
                        (5, 10, 7)
                    );
                    assert_eq!(
                        (h.frequency_scale, h.alter_scale, h.noise_bands),
                        if extra1 { (1, false, 3) } else { (2, true, 2) }
                    );
                    assert_eq!(
                        (
                            h.limiter_bands,
                            h.limiter_gains,
                            h.interpolate_frequency,
                            h.smoothing_mode
                        ),
                        if extra2 {
                            (3, 1, false, false)
                        } else {
                            (2, 2, true, true)
                        }
                    );
                    assert_eq!(bits.read(8).unwrap(), 255);
                }
            }
        }
    }

    #[test]
    fn impossible_payload_boundaries_do_not_consume_bits() {
        let mut bits = BitReader::new(&[0; 4]);
        bits.skip(3).unwrap();
        for end in [0, 2, 33, usize::MAX] {
            assert!(Header::read(&mut bits, end).is_err());
            assert_eq!(bits.position(), 3);
        }
    }
}
