//! Player adapter over library-owned Ogg Opus presentation.
use crate::Result;
use crate::audio::{AudioPacket, AudioStep, AudioStream, AudioTrack, EncodedPacket};
use crate::container::ogg::Limits;
use fvid_media::owned_ogg_opus as owned;
use std::{io::Read, time::Duration};
pub struct OggOpusAudioReader {
    program: owned::Program,
    decoder_configuration: Vec<u8>,
}
impl OggOpusAudioReader {
    pub fn open<R: Read>(source: R, limits: Limits) -> Result<Self> {
        Self::open_track(source, limits, 0)
    }
    pub fn open_track<R: Read>(source: R, limits: Limits, nth: usize) -> Result<Self> {
        let program = owned::Program::open_track(source, limits, nth)?;
        let decoder_configuration = program.extra_data().to_vec();
        Ok(Self {
            program,
            decoder_configuration,
        })
    }
    pub fn output_channel_mask(&self) -> Option<u32> {
        self.program.output_channel_mask()
    }
}
fn encoded(packet: owned::EncodedPacket) -> EncodedPacket {
    EncodedPacket {
        data: packet.data,
        pts: packet.pts,
        duration: packet.duration,
    }
}
impl AudioStream for OggOpusAudioReader {
    fn codec(&self) -> &str {
        self.program.codec()
    }
    fn timescale(&self) -> u32 {
        self.program.timescale()
    }
    fn sample_rate(&self) -> u32 {
        self.program.sample_rate()
    }
    fn channels(&self) -> u16 {
        self.program.channels()
    }
    fn extra_data(&self) -> &[u8] {
        self.program.extra_data()
    }
    fn make_decoder(&self) -> Result<Box<dyn crate::audio::AudioDecode>> {
        let head = crate::codec::opus_decoder::header(self.extra_data())?;
        crate::codec::make_audio_decoder(
            self.codec(),
            self.extra_data(),
            48000,
            u16::from(head.channel_count),
            0,
        )
    }
    fn reset_decoder(&mut self, decoder: &mut Box<dyn crate::audio::AudioDecode>) -> Result<()> {
        let configuration = self.extra_data().to_vec();
        if configuration != self.decoder_configuration {
            *decoder = self.make_decoder()?;
            self.decoder_configuration = configuration;
        } else {
            decoder.reset();
        }
        Ok(())
    }
    fn duration(&self) -> Option<Duration> {
        self.program.duration()
    }
    fn audio_tracks(&self) -> Vec<AudioTrack> {
        self.program
            .audio_tracks()
            .into_iter()
            .map(|t| AudioTrack {
                sample_rate: t.sample_rate,
                channels: t.channels,
                name: t.name,
                language: t.language,
            })
            .collect()
    }
    fn next_packet(&mut self) -> Result<Option<EncodedPacket>> {
        self.program.next_packet().map(|p| p.map(encoded))
    }
    fn next_step(&mut self) -> Result<Option<AudioStep>> {
        self.program.next_step().map(|step| {
            step.map(|s| match s {
                owned::AudioStep::ResetDecoder => AudioStep::ResetDecoder,
                owned::AudioStep::Encoded(p) => AudioStep::Encoded(encoded(p)),
            })
        })
    }
    fn rewind(&mut self) {
        self.program.rewind();
    }
    fn seek_to(&mut self, pts: i64) -> i64 {
        self.program.seek_to(pts)
    }
    fn preroll_target(&self) -> Option<i64> {
        self.program.preroll_target()
    }
    fn present_decoded(&self, packet: AudioPacket, source_pts: i64) -> Result<Option<AudioPacket>> {
        self.program
            .present_decoded(
                owned::AudioPacket {
                    data: packet.data,
                    pts: packet.pts,
                    timebase_num: packet.timebase_num,
                    timebase_den: packet.timebase_den,
                },
                source_pts,
            )
            .map(|p| {
                p.map(|p| AudioPacket {
                    data: p.data,
                    pts: p.pts,
                    timebase_num: p.timebase_num,
                    timebase_den: p.timebase_den,
                })
            })
    }
}
