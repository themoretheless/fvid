type NativeSpatial = super::aac_ld_mps_native::Decoder;

/// Pending ELD core state. Preparation leaves the decoder unchanged so the
/// AAC extension owner can validate and render spatial data before committing.
struct PreparedEldCore {
    used_bits: usize,
    samples: Vec<f32>,
    spatial_qmf: Option<super::aac_ld_mps_native::Subbands>,
    synthesis: Vec<super::aac_eld_synthesis::EldSynthesis>,
    full_synthesis: Vec<super::aac_eld_synthesis::EldSynthesis>,
    sbr: Option<super::aac_eld_sbr::Decoder>,
    noise: NoiseState,
}
impl NativeAacDecoder {
    fn prepare_eld_core(&self, packet: &[u8]) -> Result<PreparedEldCore> {
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
            13 => &super::aac_pce::CONFIGURATION_13_ELEMENTS,
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
        let mut spatial_qmf = None;
        if let Some(ld) = &mut ld {
            let end = bits.position() + bits.remaining();
            if ld.is_spatial() {
                let (output, used) =
                    ld.process_prefix_qmf(packet, bits.position(), end, &planar)?;
                let slots = output.first().map_or(0, Vec::len);
                let bins = output.first().and_then(|c| c.first()).map_or(0, Vec::len);
                if output.len() != usize::from(config.channels)
                    || slots == 0
                    || bins == 0
                    || output
                        .iter()
                        .any(|c| c.len() != slots || c.iter().any(|r| r.len() != bins))
                {
                    return Err(invalid("invalid shared LD-SBR QMF output geometry"));
                }
                let mut canonical = vec![vec![vec![Default::default(); bins]; output.len()]; slots];
                for (c, rows) in output.iter().enumerate() {
                    for (t, row) in rows.iter().enumerate() {
                        canonical[t][self.mapping[c]] = row.clone();
                    }
                }
                spatial_qmf = Some(canonical);
                bits.skip(used)?;
            } else {
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
        }
        if planar.len() != usize::from(config.channels) {
            return Err(invalid(
                "trailing bytes or channel mismatch after AAC ELD block",
            ));
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
        Ok(PreparedEldCore {
            used_bits: bits.position(),
            samples: output,
            spatial_qmf,
            synthesis: next,
            full_synthesis: full_next,
            sbr: ld,
            noise,
        })
    }
    fn commit_eld_core(&mut self, core: PreparedEldCore) -> Vec<f32> {
        self.eld_synthesis = core.synthesis;
        self.eld_full_synthesis = core.full_synthesis;
        self.eld_sbr = core.sbr;
        self.noise = core.noise;
        core.samples
    }
    fn decode_eld_timed(
        &mut self,
        packet: &[u8],
        pts: i64,
        duration: u64,
        source_timescale: u32,
    ) -> Result<Option<AacFrame>> {
        let core = self.prepare_eld_core(packet)?;
        let mut bits = BitReader::new(packet);
        bits.skip(core.used_bits)?;
        if let Some(spatial) = &mut self.ld_mps {
            let stamp = super::aac_ld_mps_native::Stamp {
                pts,
                duration,
                timescale: source_timescale,
            };
            let output = if let Some(qmf) = &core.spatial_qmf {
                spatial.decode_qmf(packet, core.used_bits, qmf, stamp)?
            } else {
                spatial.decode_core(packet, core.used_bits, &core.samples, stamp)?
            };
            self.commit_eld_core(core);
            return Ok(output.map(|frame| AacFrame {
                samples: frame.samples,
                pts: frame.pts,
                duration: frame.duration,
            }));
        }
        super::aac_pce::skip_er_extensions(&mut bits)?;
        if bits.remaining() > 7 {
            return Err(invalid(
                "trailing bytes or channel mismatch after AAC ELD block",
            ));
        }
        // er_raw_data_block_eld ends with byte_alignment(), not a zero-valued
        // syntax field. Only the preceding bounded extension bytes are syntax.
        bits.skip(bits.remaining())?;
        Ok(Some(AacFrame {
            samples: self.commit_eld_core(core),
            pts,
            duration,
        }))
    }
}
