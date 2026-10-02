//! Owned AAC-LC raw-data-block decoder. The caller supplies ASC and strips any
//! ADTS/container framing. Remaining tools are explicit errors, never fallbacks.
use super::aac_coupling_syntax::Coupling;
use super::aac_geometry::BandTables;
use super::{invalid, unsupported, Result};
fn default_pcm_mask(channels: u16) -> Result<u32> {
    crate::owned_wav::default_pcm_mask(channels).map_err(|e| invalid(&e))
}
use super::Error;
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
        Ok(footprint.total())
    }
}
#[cfg(test)]
mod retained_memory_tests {
    use super::*;
    #[test]
    fn occupied_coupling_slot_counts_its_independent_scratch() {
        let mut decoder = NativeAacDecoder::new(&[0x12, 0x08]).unwrap();
        let before = decoder.retained_payload_bytes().unwrap();
        // Ownership accounting only: this does not claim coupling profile acceptance.
        decoder.coupling_synthesis[0] = Some(decoder.synthesis[0].clone());
        assert_eq!(
            decoder.retained_payload_bytes().unwrap() - before,
            1024 * 58
        );
    }
    #[test]
    fn channels_and_checkpoints_count_mutable_capacity_but_share_tables() {
        let mono = NativeAacDecoder::new(&[0x12, 0x08]).unwrap();
        let stereo = NativeAacDecoder::new(&[0x12, 0x10]).unwrap();
        let state = std::mem::size_of::<LongSineSynthesis>() + 1024 * 58;
        assert_eq!(
            stereo.retained_payload_bytes().unwrap() - mono.retained_payload_bytes().unwrap(),
            state + std::mem::size_of::<usize>()
        );
        let checkpoint = mono.checkpoint();
        let added = state
            + std::mem::size_of::<usize>()
            + 16 * std::mem::size_of::<Option<LongSineSynthesis>>();
        assert_eq!(
            mono.retained_payload_bytes_with_checkpoint(Some(&checkpoint))
                .unwrap(),
            mono.retained_payload_bytes().unwrap() + added
        );
    }
}
