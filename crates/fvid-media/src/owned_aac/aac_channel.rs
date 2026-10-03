//! Owned AAC-LC individual channel parsing.
use super::Error;
use super::aac_geometry::BandTables;
use super::aac_tns_syntax as tns_syntax;
use super::{Result, invalid, unsupported};
include!("aac_channel_impl.rs");

impl ChannelData {
    /// Heap payload retained by parsed channel syntax, including spare capacity.
    /// Does not include decoder state, reconstructed spectra, allocator headers,
    /// or stack fields. Inspection itself allocates no heap memory.
    pub fn retained_payload_bytes(&self) -> Result<usize> {
        let mut footprint = super::memory::Footprint::new();
        footprint.vector(&self.info.group_lengths).map_err(Error)?;
        footprint.vector(&self.codebooks).map_err(Error)?;
        for group in &self.codebooks {
            footprint.vector(group).map_err(Error)?;
        }
        footprint.vector(&self.scales).map_err(Error)?;
        for group in &self.scales {
            footprint.vector(group).map_err(Error)?;
        }
        footprint.vector(&self.quantized).map_err(Error)?;
        if let Some(tns) = &self.tns {
            footprint.vector(&tns.windows).map_err(Error)?;
            for window in &tns.windows {
                footprint.vector(window).map_err(Error)?;
                for filter in window {
                    footprint.vector(&filter.lpc).map_err(Error)?;
                }
            }
        }
        Ok(footprint.total())
    }
}

#[cfg(test)]
mod memory_tests {
    use super::*;
    #[test]
    fn parsed_channel_accounts_nested_tns_and_unused_capacity() {
        use super::super::aac_scalefactors::BandScale;
        use std::mem::size_of;
        let group_lengths = Vec::<u8>::with_capacity(8);
        let codebooks = vec![Vec::<u8>::with_capacity(12)];
        let scales = vec![Vec::<BandScale>::with_capacity(13)];
        let quantized = Vec::<i16>::with_capacity(1024);
        let lpc = Vec::<f64>::with_capacity(12);
        let filters = vec![super::super::aac_tns::TnsFilter {
            length: 1,
            reverse: false,
            lpc,
        }];
        let windows = vec![filters];
        let expected = group_lengths.capacity()
            + codebooks.capacity() * size_of::<Vec<u8>>()
            + codebooks[0].capacity()
            + scales.capacity() * size_of::<Vec<BandScale>>()
            + scales[0].capacity() * size_of::<BandScale>()
            + quantized.capacity() * size_of::<i16>()
            + windows.capacity() * size_of::<Vec<super::super::aac_tns::TnsFilter>>()
            + windows[0].capacity() * size_of::<super::super::aac_tns::TnsFilter>()
            + windows[0][0].lpc.capacity() * size_of::<f64>();
        let channel = ChannelData {
            info: super::super::aac_ics::IcsInfo {
                sequence: super::super::aac_synthesis::WindowSequence::OnlyLong,
                shape: super::super::aac_synthesis::WindowShape::Sine,
                max_sfb: 0,
                group_lengths,
            },
            codebooks,
            scales,
            quantized,
            pulse: None,
            tns: Some(super::super::aac_tns::TnsData { windows }),
        };
        assert_eq!(channel.retained_payload_bytes().unwrap(), expected);
    }
}
