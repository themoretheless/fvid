//! Owned MP4 AVC/HEVC/AAC remux with presentation edits and DTS interleaving.
use super::{
    matroska_write::{
        self, Encoding, FileMetadata, PacketOptions, PacketWriter, TrackOptions, TrackSpec,
        VideoMetadata,
    },
    mp4::{Mp4Reader, Track},
};
use crate::{Result, invalid};
use fvid_control::{CancelFlag, ProgressEvent, ProgressHook};
use std::{
    cmp::Reverse,
    collections::BinaryHeap,
    io::{Read, Seek, Write},
};

use crate::codec::config::aac_specific_config;
include!("../../crates/fvid-media/src/owned_mp4_matroska_impl.rs");

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reordered_variable_cadence_uses_presentation_intervals() {
        let reader = Mp4Reader::open(
            std::io::Cursor::new(include_bytes!("../../tests/fixtures/video.mp4")),
            Default::default(),
        )
        .unwrap();
        let mut track = reader.tracks()[0].clone();
        let mut samples: Vec<_> = (0..track.samples.len())
            .map(|i| track.samples.get(i).unwrap())
            .collect();
        assert!(samples.len() >= 4);
        samples.truncate(4);
        for (sample, pts) in samples.iter_mut().zip([0, 100, 20, 65]) {
            sample.pts = pts;
            sample.duration = 10;
        }
        track.samples = super::super::mp4::SampleIndex::Expanded(samples);
        track.timescale = 1000;
        track.edits.clear();
        let plan = plan(&track, 1000, None).unwrap();
        assert_eq!(
            plan.packets
                .iter()
                .map(|p| (p.pts, p.duration))
                .collect::<Vec<_>>(),
            [
                (0, 20_000_000),
                (100_000_000, 10_000_000),
                (20_000_000, 45_000_000),
                (65_000_000, 35_000_000)
            ]
        );
    }
}
