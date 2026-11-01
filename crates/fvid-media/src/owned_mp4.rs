//! Owned bounded ISO BMFF/QuickTime demuxer, indexed and fragmented.
//! Packet positions remain in the media timeline; edit lists are separate.
//! Container parsing does not validate codec payloads or implement playback.
#![forbid(unsafe_code)]
use crate::owned_file_tags::FileTags;
use crate::owned_matroska::{ColourDescription, HdrMetadata};
use std::io::{Read, Seek, SeekFrom};
use std::ops::Range;
#[derive(Debug)]
pub struct Error {
    message: String,
    unsupported: bool,
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}
impl std::error::Error for Error {}
impl Error {
    /// The container was understood but its sample entry has no parser arm.
    pub fn is_unsupported(&self) -> bool {
        self.unsupported
    }
}
impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Self {
            message: e.to_string(),
            unsupported: false,
        }
    }
}
pub type Result<T> = std::result::Result<T, Error>;
pub(crate) fn invalid(message: &str) -> Error {
    Error {
        message: message.into(),
        unsupported: false,
    }
}
fn unsupported(message: &str) -> Error {
    Error {
        message: message.into(),
        unsupported: true,
    }
}
pub(crate) fn buffer(size: usize) -> Result<Vec<u8>> {
    let mut result = Vec::new();
    result
        .try_reserve_exact(size)
        .map_err(|_| invalid("frame allocation failed"))?;
    result.resize(size, 0);
    Ok(result)
}

pub(crate) fn reduce_ratio(numerator: u64, denominator: u64) -> (u32, u32) {
    if numerator == 0 || denominator == 0 {
        return (1, 1);
    }
    let (mut a, mut b) = (numerator, denominator);
    while b != 0 {
        let rest = a % b;
        a = b;
        b = rest;
    }
    match (u32::try_from(numerator / a), u32::try_from(denominator / a)) {
        (Ok(n), Ok(d)) => (n, d),
        _ => (1, 1),
    }
}

use crate::owned_codec_config::aac_specific_config as mp4_aac_specific_config;
use crate::owned_aac::config::AudioSpecificConfig as Mp4AudioSpecificConfig;
include!("owned_mp4_impl.rs");

#[cfg(test)]
mod admission_tests {
    use super::*;
    #[test]
    fn compact_description_prefix_keeps_unreadable_samples_visible() {
        let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/playback-errors/mp4-uniform-plan-scan/frames-64.mp4");
        let reader = Mp4Reader::open_indexed(std::io::BufReader::new(std::fs::File::open(source).unwrap()), Default::default()).unwrap();
        let mut track = reader.tracks().iter().find(|t| t.handler == *b"soun").unwrap().clone();
        track.samples = SampleIndex::Uniform(UniformIndex {
            bytes_per_frame: 4, duration: 1, count: 3,
            runs: vec![FrameRun { first: 0, offset: u64::MAX-4, frames: 3, description_index: 1 }],
        });
        let reference = (0..3).any(|i| track.description_for_sample(i) != track.description_for_sample(0));
        assert!(reference);
        assert_eq!(track.description_changes_before(3), reference);
        assert!(!track.description_changes_before(1));
        let count = (i64::MAX as u64 / u64::from(u32::MAX) + 2) as usize;
        track.samples = SampleIndex::Uniform(UniformIndex {
            bytes_per_frame: 1, duration: u32::MAX, count,
            runs: vec![FrameRun { first: 0, offset: 0, frames: count, description_index: 1 }],
        });
        assert!(track.description_for_sample(0).is_some());
        assert!(track.description_for_sample(count-1).is_none());
        assert!(track.description_changes_before(count));
        assert!(!track.description_changes_before(1));
    }

    #[test]
    fn index_payload_accounts_for_spare_capacity_and_both_sample_forms() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/audio/aac-native-edit.m4a");
        let mut reader = Mp4Reader::open(
            std::io::BufReader::new(std::fs::File::open(path).unwrap()),
            Limits::default(),
        )
        .unwrap();
        let before=reader.estimated_index_payload_bytes().unwrap();
        let capacity=reader.tracks[0].sample_descriptions.capacity();
        reader.tracks[0].sample_descriptions.reserve_exact(7);
        let growth=(reader.tracks[0].sample_descriptions.capacity()-capacity)*std::mem::size_of::<SampleDescription>();
        assert_eq!(reader.estimated_index_payload_bytes().unwrap(),before+growth);
        let before=reader.estimated_index_payload_bytes().unwrap();
        let capacity=reader.tracks[0].sample_descriptions[0].configuration.capacity();
        reader.tracks[0].sample_descriptions[0].configuration.reserve_exact(128);
        assert_eq!(reader.estimated_index_payload_bytes().unwrap(),before+reader.tracks[0].sample_descriptions[0].configuration.capacity()-capacity);
        let before = reader.estimated_index_payload_bytes().unwrap();
        let old = reader.tracks[0].configuration.capacity();
        reader.tracks[0].configuration.reserve_exact(16384);
        let growth = reader.tracks[0].configuration.capacity() - old;
        assert_eq!(
            reader.estimated_index_payload_bytes().unwrap(),
            before + growth
        );
        let before = reader.estimated_index_payload_bytes().unwrap();
        let old = reader.tags.comment.capacity();
        reader.tags.comment.reserve_exact(128);
        assert_eq!(
            reader.estimated_index_payload_bytes().unwrap(),
            before + reader.tags.comment.capacity() - old
        );
        let old = match &reader.tracks[0].samples {
            SampleIndex::Expanded(samples) => samples.capacity() * std::mem::size_of::<Sample>(),
            _ => panic!("fixture must have expanded samples"),
        };
        let before = reader.estimated_index_payload_bytes().unwrap();
        let runs = Vec::with_capacity(7);
        let expected = runs.capacity() * std::mem::size_of::<FrameRun>();
        reader.tracks[0].samples = SampleIndex::Uniform(UniformIndex {
            bytes_per_frame: 4,
            duration: 1,
            count: 1_000_000,
            runs,
        });
        assert_eq!(
            reader.estimated_index_payload_bytes().unwrap(),
            before - old + expected
        );
    }
}
