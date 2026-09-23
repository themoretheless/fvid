//! Audio packet extraction from MP4 containers.
//!
//! The audio reader extracts encoded packets from an AAC track and provides
//! them to a decoder. Packet timestamps remain in the track media timeline.

use crate::container::mp4::{Limits, Mp4Reader, Track};
use crate::{Result, invalid};
use std::io::{Read, Seek};

/// Audio packet with presentation timestamp.
pub struct AudioPacket {
    /// Encoded audio data (AAC frame, etc.).
    pub data: Vec<u8>,
    /// Presentation timestamp in track timescale units.
    pub pts: i64,
    /// Duration in track timescale units.
    pub duration: i64,
    /// Sample index within the track.
    pub sample_index: usize,
}

/// MP4 audio track reader. Extracts encoded packets for decoding.
pub struct Mp4AudioReader<R> {
    demuxer: Mp4Reader<R>,
    track_index: usize,
    sample_index: usize,
    packet: Vec<u8>,
}

impl<R: Read + Seek> Mp4AudioReader<R> {
    /// Open the first audio track that a decoder exists for.
    pub fn open(reader: R, limits: Limits) -> Result<Self> {
        let demuxer = Mp4Reader::open(reader, limits)?;
        let index = demuxer
            .tracks()
            .iter()
            .position(|t| t.handler == *b"soun" && matches!(&t.codec, b"mp4a"))
            .ok_or_else(|| invalid("MP4 has no supported audio track"))?;
        Self::from_demuxer(demuxer, index)
    }

    /// Open a specific audio track by index.
    pub fn from_demuxer(demuxer: Mp4Reader<R>, index: usize) -> Result<Self> {
        let track = demuxer
            .tracks()
            .get(index)
            .ok_or_else(|| invalid("MP4 track index out of range"))?;
        if track.handler != *b"soun" {
            return Err(invalid("selected MP4 track is not audio"));
        }
        Ok(Self {
            demuxer,
            track_index: index,
            sample_index: 0,
            packet: Vec::new(),
        })
    }

    /// Read the next audio packet. Returns None at end of track.
    pub fn read_packet(&mut self) -> Result<Option<AudioPacket>> {
        let track_index = self.track_index;
        let sample_index = self.sample_index;
        let track = self.track();
        let sample = match track.samples.get(sample_index) {
            Some(s) => s,
            None => return Ok(None),
        };
        let pts = sample.pts;
        let duration = sample.duration as i64;

        self.demuxer.read_packet(track_index, sample_index, &mut self.packet)?;

        let packet = AudioPacket {
            data: self.packet.clone(),
            pts,
            duration,
            sample_index,
        };

        self.sample_index += 1;
        Ok(Some(packet))
    }

    /// The audio track metadata.
    pub fn track(&self) -> &Track {
        &self.demuxer.tracks()[self.track_index]
    }

    /// Restart from the first sample.
    pub fn rewind(&mut self) {
        self.demuxer.invalidate_position();
        self.sample_index = 0;
        self.packet.clear();
    }

    /// Seek to the sample with the greatest PTS at or before `pts`.
    pub fn seek(&mut self, pts: i64) -> i64 {
        let track = self.track();
        let index = track
            .samples
            .iter()
            .enumerate()
            .filter(|(_, s)| s.pts <= pts)
            .max_by_key(|(_, s)| s.pts)
            .map_or(0, |(i, _)| i);
        let result_pts = track.samples.get(index).map_or(0, |s| s.pts);
        self.rewind();
        self.sample_index = index;
        result_pts
    }
}

/// Packets are timestamped in the *track* timescale, which differs from the
/// movie timescale in most files (sample rate versus a nominal 1000 Hz).
impl<R: Read + Seek + Send> crate::audio::AudioStream for Mp4AudioReader<R> {
    fn codec(&self) -> &str {
        std::str::from_utf8(&self.track().codec).unwrap_or("")
    }

    fn timescale(&self) -> u32 {
        self.track().timescale
    }

    fn sample_rate(&self) -> u32 {
        self.track().sample_rate
    }

    fn channels(&self) -> u16 {
        self.track().channels
    }

    fn extra_data(&self) -> &[u8] {
        &self.track().configuration
    }

    fn next_packet(&mut self) -> Result<Option<crate::audio::EncodedPacket>> {
        Ok(Mp4AudioReader::read_packet(self)?.map(|p| crate::audio::EncodedPacket {
            data: p.data,
            pts: p.pts,
            duration: p.duration,
        }))
    }

    fn rewind(&mut self) {
        Mp4AudioReader::rewind(self);
    }

    fn seek_to(&mut self, pts: i64) -> i64 {
        Mp4AudioReader::seek(self, pts)
    }
}
