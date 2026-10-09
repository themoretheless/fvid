//! Owned AAC-LC raw-data-block decoder. The caller supplies ASC and strips any
//! ADTS/container framing. Remaining tools are explicit errors, never fallbacks.
use super::aac_coupling_syntax::Coupling;
use super::aac_geometry::BandTables;
use super::{invalid, unsupported, Result};
fn default_pcm_mask(channels: u16) -> Result<u32> {
    crate::owned_wav::default_pcm_mask(channels).map_err(|e| invalid(&e))
}
use super::Error;
use super::{aac_sbr_history as sbr_history, aac_sbr_dsp as sbr_dsp, bits::BitReader as SbrBitReader};
fn ltp_data_for_channel(data: &super::aac_ltp_syntax::LtpData) -> std::borrow::Cow<'_, super::aac_ltp_syntax::LtpData> {
    std::borrow::Cow::Borrowed(data)
}
include!("aac_native_impl.rs");

#[cfg(test)]
mod channel_window_tests {
    use super::*;
    #[test]
    fn all_standard_channels_share_immutable_synthesis_windows() {
        for configuration in 1..=7 {
            let decoder = NativeAacDecoder::new(&[0x12, configuration << 3]).unwrap();
            assert_eq!(
                decoder.synthesis.len(),
                usize::from(decoder.config.channels)
            );
            for channel in &decoder.synthesis {
                assert!(decoder.synthesis[0].shares_windows_with(channel));
            }
        }
    }
}

impl NativeAacDecoder {
    /// Retained heap allocation payload, including spare vector capacity and
    /// uniquely owned shared tables. Excludes allocator/Arc control headers,
    /// stack objects, packet temporaries and caller-owned output. This is not
    /// a peak decode/export budget or process RSS measurement.
    pub fn retained_payload_bytes(&self) -> Result<usize> {
        self.retained_payload_bytes_with_checkpoint(None)
    }
    /// Joint retained payload of this decoder and one optional checkpoint.
    /// Shared windows/IMDCT data are counted once across channels and snapshots.
    /// Successful inspection allocates no heap memory; its bounded workspace
    /// is on stack. Error diagnostics may allocate.
    pub fn retained_payload_bytes_with_checkpoint(
        &self,
        checkpoint: Option<&AacCheckpoint>,
    ) -> Result<usize> {
        fn visit(
            synthesis: &Vec<LongSineSynthesis>,
            coupling: &Vec<Option<LongSineSynthesis>>,
            mapping: &Vec<usize>,
            program: Option<&super::aac_pce::ProgramConfig>,
            footprint: &mut super::memory::Footprint,
        ) -> std::result::Result<(), String> {
            footprint.vector(synthesis)?;
            footprint.vector(coupling)?;
            footprint.vector(mapping)?;
            for state in synthesis
                .iter()
                .chain(coupling.iter().filter_map(|state| state.as_ref()))
            {
                state.visit_retained(footprint)?;
            }
            if let Some(program) = program {
                footprint.vector(&program.elements)?;
                footprint.vector(&program.associated_data)?;
                footprint.vector(&program.coupling)?;
                footprint.vector(&program.comment)?;
            }
            Ok(())
        }
        let mut footprint = super::memory::Footprint::new();
        footprint.vector(&self.ltp_synthesis).map_err(|e|invalid(&e))?;
        for state in &self.ltp_synthesis {state.visit_retained(&mut footprint).map_err(|e|invalid(&e))?;}
        footprint.vector(&self.ltp_coupling_synthesis).map_err(|e|invalid(&e))?;
        for state in self.ltp_coupling_synthesis.iter().flatten() {state.visit_retained(&mut footprint).map_err(|e|invalid(&e))?;}
        if let Some(saved)=checkpoint {
            footprint.vector(&saved.ltp_coupling_synthesis).map_err(|e|invalid(&e))?;
            for state in saved.ltp_coupling_synthesis.iter().flatten() {state.visit_retained(&mut footprint).map_err(|e|invalid(&e))?;}
            footprint.vector(&saved.ltp_synthesis).map_err(|e|invalid(&e))?;
            for state in &saved.ltp_synthesis {state.visit_retained(&mut footprint).map_err(|e|invalid(&e))?;}
        }
        for states in [Some(&self.ssr_synthesis), checkpoint.map(|state| &state.ssr_synthesis)].into_iter().flatten() {
            footprint.vector(states).map_err(|e| invalid(&e))?;
            for state in states { state.visit_retained(&mut footprint).map_err(|e| invalid(&e))?; }
        }
        for states in [Some(&self.ssr_coupling_synthesis), checkpoint.map(|state| &state.ssr_coupling_synthesis)].into_iter().flatten() {
            footprint.vector(states).map_err(|e| invalid(&e))?;
            for state in states.iter().flatten() {
                footprint.add(std::mem::size_of::<super::aac_ssr_synthesis::SsrSynthesis>()).map_err(|e| invalid(&e))?;
                state.visit_retained(&mut footprint).map_err(|e| invalid(&e))?;
            }
        }
        for banks in [Some(&self.main_prediction), checkpoint.map(|state| &state.main_prediction)].into_iter().flatten() {
            footprint.vector(banks).map_err(|e| invalid(&e))?;
            for bank in banks.iter().flatten() { footprint.add(bank.retained_payload_bytes()).map_err(|e| invalid(&e))?; }
        }
        for tags in [Some(&self.ssr_alignment_tags), checkpoint.map(|state| &state.ssr_alignment_tags)].into_iter().flatten() {
            footprint.vector(tags).map_err(|e| invalid(&e))?;
        }
        for alignment in [self.ssr_alignment.as_ref(), checkpoint.and_then(|state| state.ssr_alignment.as_ref())].into_iter().flatten() {
            footprint.add(alignment.retained_payload_bytes()?).map_err(|e| invalid(&e))?;
        }
        visit(
            &self.synthesis,
            &self.coupling_synthesis,
            &self.mapping,
            self.program.as_ref(),
            &mut footprint,
        )
        .map_err(|e| invalid(&e))?;
        if let Some(checkpoint) = checkpoint {
            visit(
                &checkpoint.synthesis,
                &checkpoint.coupling_synthesis,
                &checkpoint.mapping,
                checkpoint.program.as_ref(),
                &mut footprint,
            )
            .map_err(|e| invalid(&e))?;
        }
        for elements in [Some(&self.sbr_elements), checkpoint.map(|state| &state.sbr_elements)].into_iter().flatten() {
            footprint.vector(elements).map_err(|e| invalid(&e))?;
            for element in elements.iter().flatten() {
                element.stream.visit_retained(&mut footprint).map_err(|e| invalid(&e))?;
                element.dsp.visit_retained(&mut footprint).map_err(|e| invalid(&e))?;
            }
        }
        for queue in [Some(&self.ssr_pending_duration),checkpoint.map(|s|&s.ssr_pending_duration)].into_iter().flatten() {
            for packet in queue.iter() {
                if let Some(sbr)=&packet.sbr {
                    footprint.vector(&sbr.groups).map_err(|e|invalid(&e))?;
                    footprint.vector(&sbr.mapping).map_err(|e|invalid(&e))?;
                    footprint.vector(&sbr.couplings).map_err(|e|invalid(&e))?;
                    for (_,frame) in &sbr.couplings {
                        if let Some(frame)=frame {footprint.add(frame.retained_payload_bytes()?).map_err(|e|invalid(&e))?;}
                    }
                    for (_,_,frame) in &sbr.groups {
                        if let Some(frame)=frame {footprint.add(frame.retained_payload_bytes()?).map_err(|e|invalid(&e))?;}
                    }
                }
            }
        }
        Ok(footprint.total())
    }
}
#[cfg(test)]
mod retained_memory_tests {
    use super::*;
    fn scratch_payload(samples: usize) -> usize {
        (3 * samples + samples / 4) * std::mem::size_of::<f64>()
            + (2 * samples - 1).next_power_of_two() * std::mem::size_of::<[f64; 2]>()
    }
    #[test]
    fn occupied_coupling_slot_counts_its_independent_scratch() {
        for (samples, flag) in [(1024, 0), (960, 4)] {
            let mut decoder = NativeAacDecoder::new(&[0x12, 0x08 | flag]).unwrap();
            assert_eq!(usize::from(decoder.config.frame_samples), samples);
            let before = decoder.retained_payload_bytes().unwrap();
            // Ownership accounting only: this does not claim coupling profile acceptance.
            decoder.coupling_synthesis[0] = Some(decoder.synthesis[0].clone());
            assert_eq!(
                decoder.retained_payload_bytes().unwrap() - before,
                scratch_payload(samples)
            );
        }
    }
    #[test]
    fn channels_and_checkpoints_count_mutable_capacity_but_share_tables() {
        for (samples, flag) in [(1024, 0), (960, 4)] {
            let mono = NativeAacDecoder::new(&[0x12, 0x08 | flag]).unwrap();
            let stereo = NativeAacDecoder::new(&[0x12, 0x10 | flag]).unwrap();
            assert_eq!(usize::from(mono.config.frame_samples), samples);
            let state = std::mem::size_of::<LongSineSynthesis>() + scratch_payload(samples);
            assert_eq!(
                stereo.retained_payload_bytes().unwrap() - mono.retained_payload_bytes().unwrap(),
                state + std::mem::size_of::<usize>()
            );
            let checkpoint = mono.checkpoint();
            let added = state
                + std::mem::size_of::<usize>()
                + 16 * std::mem::size_of::<Option<LongSineSynthesis>>();
            assert_eq!(
                mono.retained_payload_bytes_with_checkpoint(Some(&checkpoint)).unwrap(),
                mono.retained_payload_bytes().unwrap() + added
            );
        }
    }
}

#[cfg(test)]
mod element_tag_tests {
    use super::ElementTags;
    #[test]
    fn bounded_tags_distinguish_element_kinds_and_reject_repeats() {
        let mut tags = ElementTags::default();
        for kind in 0..4 {
            for tag in 0..16 {
                assert!(tags.insert(kind, tag));
                assert!(!tags.insert(kind, tag));
            }
        }
        assert!(!tags.insert(4, 0));
        assert!(!tags.insert(0, 16));
        assert_eq!(std::mem::size_of::<ElementTags>(), 8);
    }
}

#[cfg(test)]
mod he_aac_native_tests { include!("he_aac_native_tests.rs"); }

#[cfg(test)]
mod sbr_memory_tests {
    use super::*;
    #[test]
    fn retained_payload_counts_sbr_and_checkpoint_allocations_without_shared_window_duplication() {
        let cases: serde_json::Value=serde_json::from_slice(include_bytes!("../../../../tests/fixtures/playback-errors/he-aac-sbr-packets.json")).unwrap();
        let case=&cases["cases"][0];
        let asc:Vec<u8>=case["asc"].as_str().unwrap().as_bytes().chunks_exact(2).map(|b|u8::from_str_radix(std::str::from_utf8(b).unwrap(),16).unwrap()).collect();
        let mut decoder=NativeAacDecoder::new(&asc).unwrap();
        let before=decoder.retained_payload_bytes().unwrap();
        let binary=include_bytes!("../../../../tests/fixtures/playback-errors/he-aac-sbr-packets.bin");
        let frame=&case["frames"][0];let off=frame["offset"].as_u64().unwrap() as usize;
        decoder.decode(&binary[off..off+frame["bytes"].as_u64().unwrap() as usize]).unwrap();
        let after=decoder.retained_payload_bytes().unwrap();assert!(after>before+16_000);
        let checkpoint=decoder.checkpoint();
        let combined=decoder.retained_payload_bytes_with_checkpoint(Some(&checkpoint)).unwrap();
        assert!(combined>after+16_000);assert!(combined<after*2);
        decoder.reset();assert_eq!(decoder.retained_payload_bytes().unwrap(),before);
        decoder.restore(&checkpoint).unwrap();assert!(decoder.retained_payload_bytes().unwrap()>before+16_000);
    }
}

#[cfg(test)]
mod main_prediction_memory_tests {
    use super::*;
    #[test]
    fn main_predictor_allocations_and_cloned_checkpoints_are_accounted() {
        let main = NativeAacDecoder::new(&[0x0b, 0x08]).unwrap();
        let lc = NativeAacDecoder::new(&[0x13, 0x08]).unwrap();
        let bank_bytes = main.main_prediction.capacity()
            * std::mem::size_of::<Option<super::super::aac_main_predictor::MainPredictor>>()
            + main.main_prediction.iter().flatten().map(|bank| bank.retained_payload_bytes()).sum::<usize>();
        assert!(bank_bytes > 0);
        assert_eq!(main.retained_payload_bytes().unwrap(), lc.retained_payload_bytes().unwrap() + bank_bytes);
        let main_checkpoint = main.checkpoint();
        let lc_checkpoint = lc.checkpoint();
        assert_eq!(main.retained_payload_bytes_with_checkpoint(Some(&main_checkpoint)).unwrap(),
            lc.retained_payload_bytes_with_checkpoint(Some(&lc_checkpoint)).unwrap() + 2*bank_bytes);
    }
}

#[cfg(test)]
mod ltp_dispatch_tests {
    use super::*;
    use serde_json::Value;
    #[test]
    fn native_target_only_phase_controls_match_scalar_ltp_tns_pcm() {
        let manifest: Value = serde_json::from_slice(&bytes("aac-ltp-phase.json")).unwrap();
        let blob = bytes("aac-ltp-phase-control-packets.bin");
        let gold = bytes("aac-ltp-phase-control-reference.f32le");
        for case in manifest["cases"].as_array().unwrap() {
            let mut state = decoder(case["asc"].as_str().unwrap());
            for row in case["frames"].as_array().unwrap() {
                let at = row["control_offset"].as_u64().unwrap() as usize;
                let packet = &blob[at..at + row["control_bytes"].as_u64().unwrap() as usize];
                let saved = state.checkpoint();
                let pcm = state.decode(packet).unwrap();
                let at = row["control_reference_offset"].as_u64().unwrap() as usize;
                assert_eq!(pcm.len(), 1024);
                for (i, &sample) in pcm.iter().enumerate() {
                    let expected =
                        f32::from_le_bytes(gold[at + i * 4..at + i * 4 + 4].try_into().unwrap());
                    assert!(
                        (sample - expected).abs() < 1e-7,
                        "native staged TNS frame sample={i}: {sample} vs {expected}"
                    );
                }
                state.restore(&saved).unwrap();
                assert_eq!(state.decode(packet).unwrap(), pcm);
            }
        }
    }
    #[test]
    fn native_coupled_phase_videos_match_scalar_ltp_tns_pcm() {
        let manifest: Value = serde_json::from_slice(&bytes("aac-ltp-phase.json")).unwrap();
        let blob = bytes("aac-ltp-phase-packets.bin");
        let gold = bytes("aac-ltp-phase-reference.f32le");
        for case in manifest["cases"].as_array().unwrap() {
            let mut state = decoder(case["asc"].as_str().unwrap());
            let initial = state.checkpoint();
            for row in case["frames"].as_array().unwrap() {
                let at = row["offset"].as_u64().unwrap() as usize;
                let packet = &blob[at..at + row["bytes"].as_u64().unwrap() as usize];
                let saved = state.checkpoint();
                let pcm = state.decode(packet).unwrap();
                let at = row["reference_offset"].as_u64().unwrap() as usize;
                assert_eq!(pcm.len(), 1024);
                for (i, &sample) in pcm.iter().enumerate() {
                    let expected =
                        f32::from_le_bytes(gold[at + i * 4..at + i * 4 + 4].try_into().unwrap());
                    assert!(
                        (sample - expected).abs() < 1e-7,
                        "point={} sample={i}: {sample} vs {expected}",
                        case["point"]
                    );
                }
                state.restore(&saved).unwrap();
                assert_eq!(state.decode(packet).unwrap(), pcm);
            }
            state.reset();
            assert!(state.ltp_coupling_synthesis.iter().all(Option::is_none));
            state.restore(&initial).unwrap();
        }
    }
    #[test]
    fn authored_cce_gain_and_selection_videos_decode_and_replay() {
        let manifest: Value = serde_json::from_slice(&bytes("aac-ltp-coupling.json")).unwrap();
        let blob = bytes("aac-ltp-coupling-packets.bin");
        for case in manifest["cases"].as_array().unwrap() {
            let mut state = decoder(case["asc"].as_str().unwrap());
            for row in case["frames"].as_array().unwrap() {
                let at = row["offset"].as_u64().unwrap() as usize;
                let packet = &blob[at..at + row["bytes"].as_u64().unwrap() as usize];
                let saved = state.checkpoint();
                let pcm = state.decode(packet).unwrap();
                assert_eq!(pcm.len(), 1024 * usize::from(state.config.channels));
                assert!(pcm.iter().all(|x| x.is_finite()));
                state.restore(&saved).unwrap();
                assert_eq!(state.decode(packet).unwrap(), pcm);
            }
        }
    }
    #[test]
    fn independent_cce_late_routing_failure_preserves_slots_histories_and_memory() {
        let manifest: Value = serde_json::from_slice(&bytes("aac-ltp-phase.json")).unwrap();
        let case = &manifest["cases"][2];
        let blob = bytes("aac-ltp-phase-packets.bin");
        let bad = bytes("aac-ltp-phase-absent-target.bin");
        let mut state = decoder(case["asc"].as_str().unwrap());
        for row in case["frames"].as_array().unwrap() {
            let at = row["offset"].as_u64().unwrap() as usize;
            let packet = &blob[at..at + row["bytes"].as_u64().unwrap() as usize];
            let saved = state.checkpoint();
            let before = state.retained_payload_bytes().unwrap();
            let noise = state.noise.clone();
            assert!(
                state
                    .decode(&bad)
                    .unwrap_err()
                    .to_string()
                    .contains("AAC coupling target is absent")
            );
            assert_eq!(state.retained_payload_bytes().unwrap(), before);
            assert_eq!(state.noise, noise);
            assert_eq!(
                state
                    .ltp_coupling_synthesis
                    .iter()
                    .map(Option::is_some)
                    .collect::<Vec<_>>(),
                saved
                    .ltp_coupling_synthesis
                    .iter()
                    .map(Option::is_some)
                    .collect::<Vec<_>>()
            );
            let pcm = state.decode(packet).unwrap();
            state.restore(&saved).unwrap();
            assert_eq!(state.decode(packet).unwrap(), pcm);
        }
    }
    #[test]
    fn native_ltp_long_short_transitions_and_960_geometry_match_scalar_pcm() {
        let manifest: Value = serde_json::from_slice(&bytes("aac-ltp-transitions.json")).unwrap();
        let blob = bytes("aac-ltp-transitions-packets.bin");
        let gold = bytes("aac-ltp-transitions-reference.f32le");
        let wrong = bytes("aac-ltp-transitions-stale-short-reference.f32le");
        for case in manifest["cases"].as_array().unwrap() {
            let mut state = decoder(case["asc"].as_str().unwrap());
            let initial = state.checkpoint();
            let n = case["n"].as_u64().unwrap() as usize;
            let mut first = Vec::new();
            let mut stale_peak = 0f32;
            for row in case["frames"].as_array().unwrap() {
                let at = row["offset"].as_u64().unwrap() as usize;
                let packet = &blob[at..at + row["bytes"].as_u64().unwrap() as usize];
                let saved = state.checkpoint();
                let pcm = state.decode(packet).unwrap();
                let at = row["reference_offset"].as_u64().unwrap() as usize;
                assert_eq!(pcm.len(), n);
                for (i, &sample) in pcm.iter().enumerate() {
                    let expected =
                        f32::from_le_bytes(gold[at + i * 4..at + i * 4 + 4].try_into().unwrap());
                    let stale =
                        f32::from_le_bytes(wrong[at + i * 4..at + i * 4 + 4].try_into().unwrap());
                    stale_peak = stale_peak.max((sample - stale).abs());
                    assert!(
                        (sample - expected).abs() < 1e-7,
                        "n={n} seq={} sample={i}: {sample} vs {expected}",
                        row["sequence"]
                    );
                }
                state.restore(&saved).unwrap();
                assert_eq!(state.decode(packet).unwrap(), pcm);
                first.push(pcm);
            }
            assert!(
                stale_peak > 1e-6,
                "fixture must detect stale short-frame history: {stale_peak}"
            );
            state.reset();
            state.restore(&initial).unwrap();
            for (row, expected) in case["frames"].as_array().unwrap().iter().zip(first) {
                let at = row["offset"].as_u64().unwrap() as usize;
                assert_eq!(
                    state
                        .decode(&blob[at..at + row["bytes"].as_u64().unwrap() as usize])
                        .unwrap(),
                    expected
                );
            }
        }
    }
    fn bytes(name: &str) -> Vec<u8> {
        std::fs::read(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../tests/fixtures/playback-errors")
                .join(name),
        )
        .unwrap()
    }
    // Use authored AOT4 ASC through the public constructor, including PCE tags.
    fn decoder(asc: &str) -> NativeAacDecoder {
        let raw:Vec<u8>=asc.as_bytes().chunks_exact(2).map(|pair|u8::from_str_radix(std::str::from_utf8(pair).unwrap(),16).unwrap()).collect();
        assert_eq!(raw[0]>>3,4);
        NativeAacDecoder::new(&raw).unwrap()
    }
    fn qualify(case: &Value, blob: &[u8], reference: &[u8]) {
        let mut state = decoder(case["asc"].as_str().unwrap());
        let channels = usize::from(state.channels());
        assert!(state.synthesis.is_empty());
        assert_eq!(state.ltp_synthesis.len(), channels);
        let memory = state.retained_payload_bytes().unwrap();
        let mut first = None;
        for (frame, row) in case["frames"].as_array().unwrap().iter().enumerate() {
            let at = row["offset"].as_u64().unwrap() as usize;
            let packet = &blob[at..at + row["bytes"].as_u64().unwrap() as usize];
            let saved = state.checkpoint();
            let output = state
                .decode_timed(packet, (frame * 1024) as i64, 1024)
                .unwrap()
                .unwrap();
            assert_eq!(output.pts, (frame * 1024) as i64);
            assert_eq!(output.duration, 1024);
            assert_eq!(output.samples.len(), 1024 * channels);
            for (i, &sample) in output.samples.iter().enumerate() {
                let at = (frame * 1024 * channels + i) * 4;
                let gold = f32::from_le_bytes(reference[at..at + 4].try_into().unwrap());
                assert!(
                    (sample - gold).abs() < 1e-7,
                    "native LTP frame={frame} sample={i}: {sample} vs {gold}"
                );
            }
            if frame == 0 {
                first = Some((packet.to_vec(), output.samples.clone()));
            }
            state.restore(&saved).unwrap();
            assert_eq!(state.decode(packet).unwrap(), output.samples);
            assert_eq!(state.retained_payload_bytes().unwrap(), memory);
            // An invalid block leaves the complete LTP packet-boundary state unchanged.
            let mut bad = packet.to_vec();
            bad.extend([0, 0]);
            assert!(state.decode(&bad).is_err());
        }
        assert!(state.finish().unwrap().is_none());
        state.reset();
        let (packet, pcm) = first.unwrap();
        assert_eq!(state.decode(&packet).unwrap(), pcm);
    }
    #[test]
    fn authored_mono_packets_use_native_ltp_dispatch_and_checkpoints() {
        let manifest: Value = serde_json::from_slice(&bytes("aac-ltp-syntax.json")).unwrap();
        let blob = bytes("aac-ltp-packets.bin");
        for case in manifest["videos"].as_array().unwrap() {
            qualify(
                case,
                &blob,
                &bytes(&format!(
                    "aac-ltp-{}-external-reference.f32le",
                    case["name"].as_str().unwrap()
                )),
            );
        }
    }
    #[test]
    fn authored_stereo_packets_use_native_independent_ltp_states() {
        let manifest: Value = serde_json::from_slice(&bytes("aac-ltp-pair.json")).unwrap();
        let blob = bytes("aac-ltp-pair-packets.bin");
        for case in manifest["cases"].as_array().unwrap() {
            qualify(
                case,
                &blob,
                &bytes(&format!(
                    "aac-ltp-pair-{}-scalar-reference.f32le",
                    case["name"].as_str().unwrap()
                )),
            );
        }
    }
    #[test]
    fn native_ltp_checkpoints_count_only_histories_and_right_preparation_failure_preserves_histories()
     {
        let manifest: Value = serde_json::from_slice(&bytes("aac-ltp-pair.json")).unwrap();
        let case = &manifest["cases"][3];
        let blob = bytes("aac-ltp-pair-packets.bin");
        let mut state = decoder(case["asc"].as_str().unwrap());
        let saved = state.checkpoint();
        let expected = saved.ltp_synthesis.capacity()
            * std::mem::size_of::<super::super::aac_ltp_channel::LtpChannelCheckpoint>()
            + 2 * 5 * 1024 * std::mem::size_of::<f64>()
            + saved.ltp_coupling_synthesis.capacity()
                * std::mem::size_of::<Option<super::super::aac_ltp_channel::LtpChannelCheckpoint>>(
                )
            + saved.mapping.capacity() * std::mem::size_of::<usize>()
            + saved.coupling_synthesis.capacity()
                * std::mem::size_of::<Option<LongSineSynthesis>>();
        assert_eq!(
            state
                .retained_payload_bytes_with_checkpoint(Some(&saved))
                .unwrap()
                - state.retained_payload_bytes().unwrap(),
            expected
        );
        for row in case["frames"].as_array().unwrap().iter().take(5) {
            let at = row["offset"].as_u64().unwrap() as usize;
            state
                .decode(&blob[at..at + row["bytes"].as_u64().unwrap() as usize])
                .unwrap();
        }
        let row = &case["frames"][5];
        let at = row["offset"].as_u64().unwrap() as usize;
        let packet = &blob[at..at + row["bytes"].as_u64().unwrap() as usize];
        let saved = state.checkpoint();
        let gold = state.decode(packet).unwrap();
        state.restore(&saved).unwrap();
        // Controlled right-lane geometry failure after left preparation.
        // All preparation now precedes synthesis, so no history may advance.
        // This is a state-transaction test, not a reachable valid ASC.
        let right = state.ltp_synthesis[1].clone();
        state.ltp_synthesis[1] = super::super::aac_ltp_channel::LtpChannel::new(960).unwrap();
        assert!(
            state
                .decode(packet)
                .unwrap_err()
                .to_string()
                .contains("residual geometry")
        );
        state.ltp_synthesis[1] = right;
        assert_eq!(state.decode(packet).unwrap(), gold);
    }
}
