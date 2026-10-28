impl NativeAacDecoder {
    fn decode_eld_timed(
        &mut self,
        packet: &[u8],
        pts: i64,
        duration: u64,
    ) -> Result<Option<AacFrame>> {
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
            _ => return Err(invalid("unsupported AAC ELD configured layout")),
        };
        let mut bits = BitReader::new(packet);
        let mut noise = self.noise.clone();
        let mut channels = Vec::new();
        for &kind in elements {
            if kind == 1 {
                let pair = ChannelPair::read_eld(&mut bits, config)?;
                let (l, r) = pair.spectra_with_noise(config, &mut noise)?;
                channels.push((pair.left, l));
                channels.push((pair.right, r));
            } else {
                let c = ChannelData::read_eld(&mut bits, config, None, false)?;
                let s = c.spectrum_with_noise(config, &mut noise)?;
                channels.push((c, s));
            }
        }
        let mut next = self.eld_synthesis.clone();
        let mut full_next = self.eld_full_synthesis.clone();
        let mut full_planar = Vec::with_capacity(channels.len());
        let mut planar = Vec::with_capacity(channels.len());
        for (slot, (channel, spectrum)) in channels.into_iter().enumerate() {
            let spectrum = channel.apply_tns(config, spectrum)?;
            planar.push(next[self.mapping[slot]].synthesize_pcm(&spectrum)?);
            if !full_next.is_empty() {
                full_planar.push(full_next[self.mapping[slot]].synthesize_pcm(&spectrum)?);
            }
        }
        let mut ld = self.eld_sbr.clone();
        if let Some(ld) = &mut ld {
            let end = bits.position() + bits.remaining();
            let (output, used) = ld.process_prefix_with_full(
                packet,
                bits.position(),
                end,
                &planar,
                (!full_planar.is_empty()).then_some(full_planar.as_slice()),
            )?;
            bits.skip(used)?;
            planar = output;
        }
        super::aac_pce::skip_er_extensions(&mut bits)?;
        if bits.remaining() > 7 || planar.len() != usize::from(config.channels) {
            return Err(invalid(
                "trailing bytes or channel mismatch after AAC ELD block",
            ));
        }
        while bits.remaining() > 0 {
            if bits.bit()? {
                return Err(invalid("nonzero AAC ELD alignment bits"));
            }
        }
        let n = planar.first().map_or(0, Vec::len);
        if n == 0 || planar.iter().any(|c| c.len() != n) {
            return Err(invalid("AAC ELD output channel length mismatch"));
        }
        let mut output = vec![0.0; n * planar.len()];
        for (slot, channel) in planar.iter().enumerate() {
            for (i, &value) in channel.iter().enumerate() {
                output[i * usize::from(config.channels) + self.mapping[slot]] = value;
            }
        }
        self.eld_synthesis = next;
        self.eld_full_synthesis = full_next;
        self.eld_sbr = ld;
        self.noise = noise;
        Ok(Some(AacFrame {
            samples: output,
            pts,
            duration,
        }))
    }
}
