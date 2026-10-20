//! Bounded float32 speaker routing for a known front-left/front-right sink.
pub struct StereoRouter {
    channels: u16,
    coefficients: [[f32; 2]; 64],
    source_mask: Option<u32>,
}
impl StereoRouter {
    pub fn new(channels: u16, mask: u32) -> Result<Self, String> {
        let weights = crate::owned_pcm_channels::stereo_coefficients(channels, mask)?;
        let mut coefficients = [[0.; 2]; 64];
        for (to, from) in coefficients
            .iter_mut()
            .zip(weights.iter())
            .take(usize::from(channels))
        {
            *to = from.map(|v| v as f32);
        }
        let mut router = Self {
            channels,
            coefficients,
            source_mask: None,
        };
        router.source_mask = Some(mask);
        Ok(router)
    }
    /// Explicit emitted-PCM order, including codec positions not representable
    /// by a WAVE mask. AAC PCE admits at most 64 source channels.
    pub fn from_coefficients(weights: &[[f32; 2]]) -> Result<Self, String> {
        if weights.is_empty()
            || weights.len() > 64
            || !weights
                .iter()
                .flatten()
                .all(|v| v.is_finite() && v.abs() <= 64.)
        {
            return Err("invalid stereo routing coefficients".into());
        }
        let mut coefficients = [[0.; 2]; 64];
        coefficients[..weights.len()].copy_from_slice(weights);
        Ok(Self {
            channels: weights.len() as u16,
            coefficients,
            source_mask: None,
        })
    }
    pub fn source_mask(&self) -> Option<u32> {
        self.source_mask
    }
    pub fn input_channels(&self) -> u16 {
        self.channels
    }
    /// Preserve sample-frame count. Input is complete interleaved little-endian
    /// float32 PCM; output allocation is admitted before reserving or conversion.
    pub fn route(&self, data: &[u8], maximum_bytes: usize) -> Result<Vec<f32>, String> {
        let stride = usize::from(self.channels) * 4;
        if !data.len().is_multiple_of(stride) {
            return Err("partial decoded audio frame".into());
        }
        let samples = (data.len() / stride)
            .checked_mul(2)
            .ok_or("stereo routing size overflow")?;
        if samples
            .checked_mul(4)
            .is_none_or(|bytes| bytes > maximum_bytes)
        {
            return Err("stereo routing exceeds output limit".into());
        }
        let mut output = Vec::new();
        output
            .try_reserve_exact(samples)
            .map_err(|_| "stereo routing allocation failed")?;
        for frame in data.chunks_exact(stride) {
            let mut stereo = [0f32; 2];
            for (i, bytes) in frame.chunks_exact(4).enumerate() {
                let value = f32::from_le_bytes(bytes.try_into().unwrap());
                if !value.is_finite() {
                    return Err("non-finite decoded audio sample".into());
                }
                stereo[0] += value * self.coefficients[i][0];
                stereo[1] += value * self.coefficients[i][1];
            }
            if !stereo.iter().all(|v| v.is_finite()) {
                return Err("stereo routing overflow".into());
            }
            output.extend(stereo);
        }
        Ok(output)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn equal_channel_geometry_still_routes_rear_speakers_and_directout() {
        let input: Vec<_> = [1f32, 2.].into_iter().flat_map(f32::to_le_bytes).collect();
        let k = std::f32::consts::FRAC_1_SQRT_2;
        assert_eq!(
            StereoRouter::new(2, 0x30)
                .unwrap()
                .route(&input, 8)
                .unwrap(),
            [k, 2. * k]
        );
        assert_eq!(
            StereoRouter::new(2, 0).unwrap().route(&input, 8).unwrap(),
            [0., 0.]
        );
    }
    #[test]
    fn routing_admission_and_malformed_pcm_fail_before_output() {
        let router = StereoRouter::new(1, 4).unwrap();
        assert!(router
            .route(&1f32.to_le_bytes(), 7)
            .unwrap_err()
            .contains("output limit"));
        assert!(router.route(&[0; 3], 8).unwrap_err().contains("partial"));
        assert!(router
            .route(&f32::NAN.to_le_bytes(), 8)
            .unwrap_err()
            .contains("non-finite"));
        assert!(StereoRouter::new(1, 3).is_err());
        assert!(StereoRouter::new(0, 0).is_err());
    }
}
#[cfg(test)]
mod explicit_position_capacity {
    use super::*;
    #[test]
    fn sixty_four_coefficients_preserve_source_order_without_mask_truncation() {
        let weights: Vec<_> = (0..64)
            .map(|i| if i % 2 == 0 { [1f32, 0.] } else { [0., 1.] })
            .collect();
        let router = StereoRouter::from_coefficients(&weights).unwrap();
        assert_eq!(router.source_mask(), None);
        assert_eq!(router.input_channels(), 64);
        let data: Vec<_> = (1..=64).flat_map(|v| (v as f32).to_le_bytes()).collect();
        assert_eq!(router.route(&data, 8).unwrap(), [1024., 1056.]);
        assert!(StereoRouter::from_coefficients(&[[1., 1.]; 65]).is_err());
        assert!(StereoRouter::from_coefficients(&[[f32::INFINITY, 0.]]).is_err());
    }
}
