/// Pending ELD core state. Preparation leaves the decoder unchanged so the
/// AAC extension owner can validate and render spatial data before committing.
struct PreparedEldCore {
    used_bits: usize,
    samples: Vec<f32>,
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
    ) -> Result<Option<AacFrame>> {
        let core = self.prepare_eld_core(packet)?;
        let mut bits = BitReader::new(packet);
        bits.skip(core.used_bits)?;
        super::aac_pce::skip_er_extensions(&mut bits)?;
        if bits.remaining() > 7 {
            return Err(invalid(
                "trailing bytes or channel mismatch after AAC ELD block",
            ));
        }
        while bits.remaining() > 0 {
            if bits.bit()? {
                return Err(invalid("nonzero AAC ELD alignment bits"));
            }
        }
        Ok(Some(AacFrame {
            samples: self.commit_eld_core(core),
            pts,
            duration,
        }))
    }
}

#[cfg(test)]
mod eld_core_boundary_tests {
    use super::*;
    fn fixture(name: &str) -> Vec<u8> {
        std::fs::read(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../tests/fixtures/playback-errors/aac-eld-mps-core")
                .join(name),
        )
        .unwrap()
    }
    #[test]
    fn real_eld_core_boundary_spatial_suffix_and_late_commit() {
        for tree in 0..8 {
            let channels = if matches!(tree, 0 | 1 | 7) {
                1
            } else if matches!(tree, 2 | 3 | 4) {
                2
            } else {
                6
            };
            let asc = fixture(&format!("tree{tree}.asc"));
            let mut decoder = NativeAacDecoder::new(&asc).unwrap();
            let mut reference = NativeAacDecoder::new(&asc).unwrap();
            let golden = fixture(&format!("tree{tree}.pcm"));
            let spatial_config = super::super::aac_ld_mps_config::Config::parse(&fixture(
                &format!("tree{tree}.ssc"),
            ))
            .unwrap();
            let mut spatial = super::super::aac_ld_mps_pcm::Decoder::new(
                &spatial_config,
                super::super::aac_ld_mps_decor::Mode::Complex,
            )
            .unwrap();
            let mut golden_spatial = spatial.clone();
            let residuals = vec![
                None;
                spatial_config.tree.ott_lfe.len()
                    + usize::from(spatial_config.tree.ttt_boxes)
            ];
            for frame in 0..3 {
                let packet = fixture(&format!("tree{tree}-{frame}.au"));
                let lengths = fixture(&format!("tree{tree}-{frame}.bits"));
                let core_bits = u32::from_le_bytes(lengths[..4].try_into().unwrap()) as usize;
                let end = u32::from_le_bytes(lengths[4..].try_into().unwrap()) as usize;
                if end % 8 != 0 {
                    let mut invalid_alignment = packet.clone();
                    *invalid_alignment.last_mut().unwrap() |= 1;
                    let error = decoder
                        .decode_eld_timed(&invalid_alignment, frame as i64 * 512, 512)
                        .unwrap_err()
                        .to_string();
                    assert!(error.contains("nonzero AAC ELD alignment"), "{error}");
                }
                // Preparing, rejecting a suffix, and retrying must not advance core history.
                let abandoned = decoder.prepare_eld_core(&packet).unwrap();
                assert_eq!(abandoned.used_bits, core_bits);
                let mut bad = BitReader::new(&packet);
                bad.skip(core_bits).unwrap();
                assert!(
                    super::super::aac_ld_mps_transport::Assembler::default()
                        .read(&mut bad, core_bits + 7)
                        .is_err()
                );
                drop(abandoned);
                let pending = decoder.prepare_eld_core(&packet).unwrap();
                let expected = reference.prepare_eld_core(&packet).unwrap();
                assert_eq!(pending.samples, expected.samples);
                let offset = frame * 512 * channels * 4;
                for (i, &v) in pending.samples.iter().enumerate() {
                    let at = offset + i * 4;
                    let want = f32::from_le_bytes(golden[at..at + 4].try_into().unwrap());
                    assert!(
                        (v - want).abs() < 2e-7,
                        "tree {tree} frame {frame} sample {i}: {v} != {want}"
                    );
                }
                let mut bits = BitReader::new(&packet);
                bits.skip(pending.used_bits).unwrap();
                let message = super::super::aac_ld_mps_transport::Assembler::default()
                    .read(&mut bits, end)
                    .unwrap()
                    .unwrap();
                assert_eq!(
                    message.payload,
                    fixture(&format!("tree{tree}-{frame}.spatial"))
                );
                let native_planar: Vec<Vec<f64>> = (0..channels)
                    .map(|c| {
                        pending
                            .samples
                            .chunks_exact(channels)
                            .map(|r| f64::from(r[c]))
                            .collect()
                    })
                    .collect();
                let golden_planar: Vec<Vec<f64>> = (0..channels)
                    .map(|c| {
                        (0..512)
                            .map(|i| {
                                let at = offset + (i * channels + c) * 4;
                                f64::from(f32::from_le_bytes(
                                    golden[at..at + 4].try_into().unwrap(),
                                ))
                            })
                            .collect()
                    })
                    .collect();
                let saved_spatial = spatial.clone();
                let mut spatial_bits = BitReader::new(&message.payload);
                let rendered = spatial
                    .decode(&mut spatial_bits, &native_planar, &residuals)
                    .unwrap();
                let expected_render = golden_spatial
                    .decode(
                        &mut BitReader::new(&message.payload),
                        &golden_planar,
                        &residuals,
                    )
                    .unwrap();
                assert_eq!(
                    rendered.samples.len(),
                    512 * usize::from(spatial_config.tree.output_channels)
                );
                assert_eq!(rendered.sample_rate, 48000);
                for (&v, &want) in rendered.samples.iter().zip(&expected_render.samples) {
                    assert!(
                        v.is_finite() && (v - want).abs() < 2e-6,
                        "tree {tree}: {v} != {want}"
                    );
                }
                let mut replay_spatial = saved_spatial;
                assert_eq!(
                    replay_spatial
                        .decode(
                            &mut BitReader::new(&message.payload),
                            &native_planar,
                            &residuals
                        )
                        .unwrap(),
                    rendered
                );
                assert_eq!(replay_spatial, spatial);
                assert_eq!(bits.position(), end);
                while bits.remaining() > 0 {
                    assert!(!bits.bit().unwrap());
                }
                assert_eq!(
                    decoder.commit_eld_core(pending),
                    reference.commit_eld_core(expected)
                );
            }
            let error = NativeAacDecoder::new(&fixture(&format!("tree{tree}.spatial.asc")))
                .err()
                .unwrap()
                .to_string();
            assert!(error.contains("LD-MPS spatial reconstruction"), "{error}");
        }
    }
}
