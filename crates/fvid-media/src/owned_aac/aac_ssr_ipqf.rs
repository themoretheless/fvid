//! Owned AAC-SSR synthesis filter, ISO/IEC 14496-3:2009 4.6.12.3.4.
//! Consumes gain-compensated, overlap-added samples in four frequency bands.
//! This building block does not by itself enable SSR packet decoding.
use super::{Result, invalid};
use std::{f64::consts::PI, sync::Arc};

// Table 4.164: the other half is the reflection Q(j) = Q(95-j).
const PROTOTYPE: [f64; 48] = [
    9.7655291007575512e-05,
    1.3809589379038567e-04,
    9.8400749256623534e-05,
    -8.6671544782335723e-05,
    -4.6217998911921346e-04,
    -1.0211814095158174e-03,
    -1.6772149340010668e-03,
    -2.2533338951411081e-03,
    -2.4987888343213967e-03,
    -2.1390815966761882e-03,
    -9.5595397454597772e-04,
    1.1172111530118943e-03,
    3.9091309127348584e-03,
    6.9635703420118673e-03,
    9.5595442159478339e-03,
    1.0815766540021360e-02,
    9.8770514991715300e-03,
    6.1562567291327357e-03,
    -4.1793946063629710e-04,
    -9.2128743097707640e-03,
    -1.8830775873369020e-02,
    -2.7226498457701823e-02,
    -3.2022840857588906e-02,
    -3.0996332527754609e-02,
    -2.2656858741499447e-02,
    -6.8031113858963354e-03,
    1.5085400948280744e-02,
    3.9750993388272739e-02,
    6.2445363629436743e-02,
    7.7622327748721326e-02,
    7.9968338496132926e-02,
    6.5615493068475583e-02,
    3.3313658300882690e-02,
    -1.4691563058190206e-02,
    -7.2307890475334147e-02,
    -1.2993222541703875e-01,
    -1.7551641029040532e-01,
    -1.9626543957670528e-01,
    -1.8073330670215029e-01,
    -1.2097653136035738e-01,
    -1.4377370758549035e-02,
    1.3522730742860303e-01,
    3.1737852699301633e-01,
    5.1590021798482233e-01,
    7.1080020379761377e-01,
    8.8090632488444798e-01,
    1.0068321641150089e+00,
    1.0737914947736096e+00,
];
fn prototype(j: usize) -> f64 {
    PROTOTYPE[j.min(95 - j)]
}

#[derive(Clone)]
pub struct SsrIpqf {
    // phase, band, delay in quarter-rate samples
    coefficients: Arc<[[[f64; 24]; 4]; 4]>,
    history: [[f64; 4]; 24],
    cursor: usize,
}
impl Default for SsrIpqf {
    fn default() -> Self {
        Self::new()
    }
}
impl SsrIpqf {
    pub fn new() -> Self {
        let coefficients = std::array::from_fn(|phase| {
            std::array::from_fn(|band| {
                std::array::from_fn(|lag| {
                    let j = 4 * lag + phase;
                    prototype(j)
                        * (((2 * band + 1) as f64 * (2 * j as isize - 3) as f64 * PI) / 16.0).cos()
                })
            })
        });
        Self {
            coefficients: Arc::new(coefficients),
            history: [[0.0; 4]; 24],
            cursor: 0,
        }
    }
    pub fn reset(&mut self) {
        self.history.fill([0.0; 4]);
        self.cursor = 0;
    }

    /// One row contains simultaneous low-to-high band samples. Emits four PCM
    /// samples per row. No allocations; invalid input leaves history/output intact.
    /// A clone is an independent checkpoint with shared immutable coefficients.
    pub fn synthesize(&mut self, bands: &[[f64; 4]], pcm: &mut [f64]) -> Result<()> {
        if bands.len().checked_mul(4) != Some(pcm.len())
            || bands.iter().flatten().any(|v| !v.is_finite())
        {
            return Err(invalid("AAC SSR IPQF invalid input or output size"));
        }
        for (row, output) in bands.iter().zip(pcm.chunks_exact_mut(4)) {
            self.history[self.cursor] = *row;
            for (phase, sample) in output.iter_mut().enumerate() {
                let mut sum = 0.0;
                for lag in 0..24 {
                    let delayed = &self.history[(self.cursor + 24 - lag) % 24];
                    for (band, value) in delayed.iter().enumerate() {
                        sum += self.coefficients[phase][band][lag] * value;
                    }
                }
                *sample = sum;
            }
            self.cursor = (self.cursor + 1) % 24;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    // Literal upsampling and convolution, independent of the polyphase ring.
    fn direct(rows: &[[f64; 4]]) -> Vec<f64> {
        (0..4 * rows.len())
            .map(|n| {
                let mut sum = 0.0;
                for band in 0..4 {
                    for j in 0..96 {
                        if n >= j && (n - j) % 4 == 0 {
                            sum += prototype(j)
                                * (((2 * band + 1) as f64 * (2 * j as isize - 3) as f64 * PI)
                                    / 16.0)
                                    .cos()
                                * rows[(n - j) / 4][band];
                        }
                    }
                }
                sum
            })
            .collect()
    }
    #[test]
    fn nonzero_bands_match_literal_upsampling_convolution_across_packets() {
        let rows: Vec<_> = (0..513)
            .map(|i| std::array::from_fn(|b| ((i * 11 + b * 7) as f64 * 0.037).sin()))
            .collect();
        let expected = direct(&rows);
        let mut filter = SsrIpqf::new();
        let mut actual = vec![0.0; expected.len()];
        // Irregular boundaries deliberately cross the 24-row history wrap.
        let mut start = 0;
        for end in [1, 23, 24, 25, 31, 256, 513] {
            filter
                .synthesize(&rows[start..end], &mut actual[4 * start..4 * end])
                .unwrap();
            start = end;
        }
        for (a, e) in actual.iter().zip(&expected) {
            assert!((a - e).abs() < 2e-14, "{a} vs {e}");
        }
    }
    #[test]
    fn every_band_impulse_has_all_96_taps_and_no_extra_tail() {
        for band in 0..4 {
            let mut rows = [[0.0; 4]; 25];
            rows[0][band] = 1.0;
            let mut output = [0.0; 100];
            SsrIpqf::new().synthesize(&rows, &mut output).unwrap();
            assert_eq!(output.to_vec(), direct(&rows));
            assert!(output[..96].iter().all(|v| v.abs() > 0.0));
            assert_eq!(output[96..], [0.0; 4]);
        }
    }
    #[test]
    fn invalid_input_checkpoint_and_reset_preserve_streaming_state() {
        let mut filter = SsrIpqf::new();
        filter
            .synthesize(&[[1.0, 2.0, 3.0, 4.0]; 27], &mut [0.0; 108])
            .unwrap();
        let mut checkpoint = filter.clone();
        let mut output = [17.0; 4];
        assert!(filter.synthesize(&[[f64::NAN; 4]], &mut output).is_err());
        assert!(filter.synthesize(&[[0.0; 4]], &mut output[..3]).is_err());
        assert_eq!(output, [17.0; 4]);
        filter.synthesize(&[[0.0; 4]], &mut output).unwrap();
        let mut expected = [0.0; 4];
        checkpoint.synthesize(&[[0.0; 4]], &mut expected).unwrap();
        assert_eq!(output, expected);
        assert!(output.iter().any(|v| *v != 0.0));
        filter.reset();
        filter.synthesize(&[[0.0; 4]], &mut output).unwrap();
        assert_eq!(output, [0.0; 4]);
        filter.synthesize(&[], &mut []).unwrap();
    }
}
