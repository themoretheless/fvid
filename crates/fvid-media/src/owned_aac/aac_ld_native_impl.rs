impl NativeAacDecoder {
    fn decode_ld_timed(
        &mut self,
        packet: &[u8],
        pts: i64,
        duration: u64,
    ) -> Result<Option<AacFrame>> {
        use super::aac_ld_synthesis::LdWindowShape;
        let config = &self.config;
        let elements: &[u8] = match config.channel_configuration {
            1 => &[0],
            2 => &[1],
            3 => &[0, 1],
            4 => &[0, 1, 0],
            5 => &[0, 1, 1],
            6 => &[0, 1, 1, 3],
            7 | 12 => &[0, 1, 1, 1, 3],
            11 => &[0, 1, 1, 0, 3],
            14 => &[0, 1, 1, 3, 1],
            _ => return Err(invalid("unsupported AAC LD configured layout")),
        };
        let bands =
            super::aac_ld_bands::LdBands::new(config.sample_rate, config.frame_samples as usize)?;
        let mut bits = BitReader::new(packet);
        let mut noise = self.noise.clone();
        let mut channels = Vec::with_capacity(usize::from(config.channels));
        for element in elements {
            bits.read(4)?; // ER tags are signaled but configured order owns placement.
            if *element == 1 {
                let (pair, prediction) = ChannelPair::read_ld(&mut bits, config)?;
                let (left, right) = pair.spectra_with_noise(config, &mut noise)?;
                let [before, after] = prediction;
                channels.push((pair.left, left, before));
                channels.push((pair.right, right, after));
            } else {
                let (channel, prediction) = ChannelData::read_ld(&mut bits, config, false)?;
                let spectrum = channel.spectrum_with_noise(config, &mut noise)?;
                channels.push((channel, spectrum, prediction));
            }
        }
        if channels.len() != usize::from(config.channels) || bits.remaining() > 7 {
            return Err(invalid(
                "trailing bytes or channel mismatch after AAC LD block",
            ));
        }
        // Whole-packet transaction: a later channel failure cannot advance an
        // earlier channel's PCM, lag, window, or noise history.
        let mut next = self.ld_synthesis.clone();
        let n = usize::from(config.frame_samples);
        let mut output = vec![0.; n * channels.len()];
        for (slot, (channel, residual, mut prediction)) in channels.into_iter().enumerate() {
            let target = self.mapping[slot];
            let shape = if channel.info.shape == super::aac_synthesis::WindowShape::Sine {
                LdWindowShape::Sine
            } else {
                LdWindowShape::LowOverlap
            };
            let limit = bands.tns_max_bands.min(usize::from(channel.info.max_sfb));
            if let Some(data) = &mut prediction { channel.suppress_ltp_noise(&mut data.used); }
            let pcm = next[target].process(
                residual,
                prediction.as_ref(),
                shape,
                bands.offsets,
                limit,
                channel.tns.as_ref(),
            )?;
            for (i, value) in pcm.iter().enumerate() {
                let value = *value as f32;
                if !value.is_finite() {
                    return Err(invalid("AAC LD PCM exceeds finite f32 output"));
                }
                output[i * usize::from(config.channels) + target] = value;
            }
        }
        self.ld_synthesis = next;
        self.noise = noise;
        Ok(Some(AacFrame {
            samples: output,
            pts,
            duration,
        }))
    }
}
