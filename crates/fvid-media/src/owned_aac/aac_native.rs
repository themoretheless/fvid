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
        for stream in [self.sbr_stream.as_ref(), checkpoint.and_then(|state| state.sbr_stream.as_ref())].into_iter().flatten() {
            stream.visit_retained(&mut footprint).map_err(|e| invalid(&e))?;
        }
        for dsp in [self.sbr_dsp.as_ref(), checkpoint.and_then(|state| state.sbr_dsp.as_ref())].into_iter().flatten() {
            dsp.visit_retained(&mut footprint).map_err(|e| invalid(&e))?;
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
