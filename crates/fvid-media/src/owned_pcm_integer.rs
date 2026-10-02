//! Integer PCM normalization and saturating quantization, independent of libav.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    U8,
    I16,
    I32,
    /// Transforming S64 uses double precision; identity copies must retain raw bytes.
    I64,
}
impl Format {
    pub fn bytes(self) -> usize {
        match self {
            Self::U8 => 1,
            Self::I16 => 2,
            Self::I32 => 4,
            Self::I64 => 8,
        }
    }
    pub fn decode(self, input: &[u8]) -> Result<f64, String> {
        if input.len() != self.bytes() {
            return Err("incorrect integer PCM sample size".into());
        }
        Ok(match self {
            Self::U8 => (f64::from(input[0]) - 128.) / 128.,
            Self::I16 => f64::from(i16::from_le_bytes(input.try_into().unwrap())) / 32768.,
            Self::I32 => f64::from(i32::from_le_bytes(input.try_into().unwrap())) / 2147483648.,
            Self::I64 => {
                i64::from_le_bytes(input.try_into().unwrap()) as f64 / 9223372036854775808.
            }
        })
    }
    pub fn encode(self, sample: f64, output: &mut [u8]) -> Result<(), String> {
        if output.len() != self.bytes() || !sample.is_finite() {
            return Err("invalid integer PCM output".into());
        }
        match self {
            Self::U8 => output[0] = (sample * 128. + 128.).round_ties_even().clamp(0., 255.) as u8,
            Self::I16 => output.copy_from_slice(
                &((sample * 32768.)
                    .round_ties_even()
                    .clamp(i16::MIN as f64, i16::MAX as f64) as i16)
                    .to_le_bytes(),
            ),
            Self::I32 => output.copy_from_slice(
                &((sample * 2147483648.)
                    .round_ties_even()
                    .clamp(i32::MIN as f64, i32::MAX as f64) as i32)
                    .to_le_bytes(),
            ),
            Self::I64 => output.copy_from_slice(
                &((sample * 9223372036854775808.).round_ties_even() as i64).to_le_bytes(),
            ),
        }
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn integer_roundtrip_is_exact_at_boundaries_and_interior() {
        for format in [Format::U8, Format::I16, Format::I32] {
            let values: Vec<Vec<u8>> = match format {
                Format::U8 => (0..=255).map(|v| vec![v]).collect(),
                Format::I16 => [i16::MIN, -32767, -1, 0, 1, 12345, i16::MAX]
                    .into_iter()
                    .map(|v| v.to_le_bytes().to_vec())
                    .collect(),
                Format::I32 => [i32::MIN, i32::MIN + 1, -1, 0, 1, 123456789, i32::MAX]
                    .into_iter()
                    .map(|v| v.to_le_bytes().to_vec())
                    .collect(),
                Format::I64 => {
                    unreachable!("S64 transforms are tested for double precision separately")
                }
            };
            for bytes in values {
                let mut output = vec![0; format.bytes()];
                format
                    .encode(format.decode(&bytes).unwrap(), &mut output)
                    .unwrap();
                assert_eq!(output, bytes);
            }
        }
    }
    #[test]
    fn saturation_ties_and_invalid_samples() {
        let mut output = [0; 2];
        Format::I16.encode(2., &mut output).unwrap();
        assert_eq!(output, i16::MAX.to_le_bytes());
        Format::I16.encode(-2., &mut output).unwrap();
        assert_eq!(output, i16::MIN.to_le_bytes());
        Format::I16.encode(0.5 / 32768., &mut output).unwrap();
        assert_eq!(output, 0i16.to_le_bytes());
        Format::I16.encode(1.5 / 32768., &mut output).unwrap();
        assert_eq!(output, 2i16.to_le_bytes());
        assert!(Format::I16.encode(f64::NAN, &mut output).is_err());
        assert!(Format::I16.decode(&[0]).is_err());
    }
}

#[cfg(test)]
mod s64_tests {
    use super::Format;
    #[test]
    fn double_precision_s64_quantization_saturates_and_rounds_ties() {
        let mut output = [0; 8];
        for (sample, expected) in [
            (-2.0, i64::MIN),
            (-1.0, i64::MIN),
            (1.0, i64::MAX),
            (2.0, i64::MAX),
            (0.0, 0),
            (0.5, 1i64 << 62),
            (-0.5, -(1i64 << 62)),
            (0.5 / 9223372036854775808., 0),
            (1.5 / 9223372036854775808., 2),
            (-1.5 / 9223372036854775808., -2),
        ] {
            Format::I64.encode(sample, &mut output).unwrap();
            assert_eq!(i64::from_le_bytes(output), expected);
        }
        assert_eq!(Format::I64.decode(&i64::MIN.to_le_bytes()).unwrap(), -1.0);
        assert_eq!(
            Format::I64.decode(&(1i64 << 62).to_le_bytes()).unwrap(),
            0.5
        );
        assert!(Format::I64.encode(f64::NAN, &mut output).is_err());
        assert!(Format::I64.encode(f64::INFINITY, &mut output).is_err());
        assert!(Format::I64.decode(&[0; 4]).is_err());
    }
}
