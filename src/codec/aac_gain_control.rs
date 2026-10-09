//! Owned bounded AAC gain-control syntax.
use super::{aac_synthesis::WindowSequence, bits::BitReader};
use crate::Result;
include!("../../crates/fvid-media/src/owned_aac/aac_gain_control_impl.rs");

impl From<GainControl> for fvid_media::owned_aac::aac_gain_control::GainControl {
    fn from(value: GainControl) -> Self {
        Self {
            bands: value
                .bands
                .into_iter()
                .map(|band| {
                    band.into_iter()
                        .map(|window| {
                            window
                                .into_iter()
                                .map(|a| fvid_media::owned_aac::aac_gain_control::Adjustment {
                                    level: a.level,
                                    location: a.location,
                                })
                                .collect()
                        })
                        .collect()
                })
                .collect(),
        }
    }
}
