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
