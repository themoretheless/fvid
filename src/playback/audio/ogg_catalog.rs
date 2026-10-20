//! Unified player track selection over the library's Ogg audio catalogue.
use crate::{
    Result,
    audio::{AudioPacket, AudioStep, AudioStream, AudioTrack, EncodedPacket},
};
use std::{fs::File, io::BufReader, path::Path, time::Duration};
pub struct Reader {
    inner: Box<dyn AudioStream>,
    tracks: Vec<AudioTrack>,
    index: usize,
}
impl Reader {
    pub fn open(path: &Path, index: usize) -> Result<Self> {
        let ogg = fvid_media::owned_ogg_vorbis::Program::read(
            BufReader::new(File::open(path)?),
            Default::default(),
        )?;
        let catalog = fvid_media::owned_ogg_audio_catalog::from_ogg(&ogg)?;
        let selected = catalog
            .get(index)
            .ok_or_else(|| crate::invalid("selected Ogg audio track is absent"))?;
        let inner: Box<dyn AudioStream> =
            if fvid_media::owned_ogg_audio_catalog::needs_joint_program(&ogg, index)? {
                Box::new(Joint {
                    program: fvid_media::owned_ogg_audio_program::Program::from_ogg(ogg, index)?,
                })
            } else {
                drop(ogg);
                let source = BufReader::new(File::open(path)?);
                match selected.coding {
                    fvid_media::owned_ogg::Coding::Vorbis => {
                        Box::new(crate::playback_ogg_audio::OggAudioReader::open_track(
                            source,
                            Default::default(),
                            selected.codec_ordinal,
                        )?)
                    }
                    fvid_media::owned_ogg::Coding::Opus => {
                        Box::new(crate::playback_ogg_opus::OggOpusAudioReader::open_track(
                            source,
                            Default::default(),
                            selected.codec_ordinal,
                        )?)
                    }
                    _ => return Err(crate::unsupported("unsupported Ogg audio catalogue coding")),
                }
            };
        let mut tracks: Vec<AudioTrack> = catalog
            .into_iter()
            .map(|t| AudioTrack {
                sample_rate: t.audio.sample_rate,
                channels: t.audio.channels,
                name: t.audio.name,
                language: t.audio.language,
            })
            .collect();
        tracks[index].sample_rate = inner.sample_rate();
        tracks[index].channels = inner.channels();
        Ok(Self {
            inner,
            tracks,
            index,
        })
    }
}
impl AudioStream for Reader {
    fn codec(&self) -> &str {
        self.inner.codec()
    }
    fn timescale(&self) -> u32 {
        self.inner.timescale()
    }
    fn sample_rate(&self) -> u32 {
        self.inner.sample_rate()
    }
    fn channels(&self) -> u16 {
        self.inner.channels()
    }
    fn bits_per_sample(&self) -> u16 {
        self.inner.bits_per_sample()
    }
    fn duration(&self) -> Option<Duration> {
        self.inner.duration()
    }
    fn extra_data(&self) -> &[u8] {
        self.inner.extra_data()
    }
    fn audio_tracks(&self) -> Vec<AudioTrack> {
        self.tracks.clone()
    }
    fn audio_track_index(&self) -> usize {
        self.index
    }
    fn make_decoder(&self) -> Result<Box<dyn crate::audio::AudioDecode>> {
        self.inner.make_decoder()
    }
    fn reset_decoder(&mut self, decoder: &mut Box<dyn crate::audio::AudioDecode>) -> Result<()> {
        self.inner.reset_decoder(decoder)
    }
    fn next_packet(&mut self) -> Result<Option<EncodedPacket>> {
        self.inner.next_packet()
    }
    fn next_step(&mut self) -> Result<Option<AudioStep>> {
        self.inner.next_step()
    }
    fn rewind(&mut self) {
        self.inner.rewind()
    }
    fn seek_to(&mut self, target: i64) -> i64 {
        self.inner.seek_to(target)
    }
    fn preroll_target(&self) -> Option<i64> {
        self.inner.preroll_target()
    }
    fn resume_preroll(&mut self, target: i64) -> bool {
        self.inner.resume_preroll(target)
    }
    fn present_decoded(&self, packet: AudioPacket, source: i64) -> Result<Option<AudioPacket>> {
        self.inner.present_decoded(packet, source)
    }
    fn packet_sample_limit(&self, duration: u64) -> Result<Option<usize>> {
        self.inner.packet_sample_limit(duration)
    }
    fn drain_decoded_at_eof(&self) -> bool {
        self.inner.drain_decoded_at_eof()
    }
    fn validate_eof(&self) -> Result<()> {
        self.inner.validate_eof()
    }
}

/// Direct PCM steps keep codec replacement inside the library-owned program.
struct Joint {
    program: fvid_media::owned_ogg_audio_program::Program,
}
impl AudioStream for Joint {
    fn codec(&self) -> &str {
        self.program.codec()
    }
    fn timescale(&self) -> u32 {
        self.program.sample_rate()
    }
    fn sample_rate(&self) -> u32 {
        self.program.sample_rate()
    }
    fn channels(&self) -> u16 {
        self.program.channels()
    }
    fn bits_per_sample(&self) -> u16 {
        32
    }
    fn duration(&self) -> Option<Duration> {
        Some(crate::audio::sample_duration(
            u64::try_from(self.program.end_samples()).ok()?, self.sample_rate(),
        ))
    }
    fn extra_data(&self) -> &[u8] {
        &[]
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
    fn audio_track_index(&self) -> usize {
        self.program.audio_track_index()
    }
    fn make_decoder(&self) -> Result<Box<dyn crate::audio::AudioDecode>> {
        crate::codec::make_audio_decoder(
            "A_PCM/FLOAT/IEEE",
            &[],
            self.sample_rate(),
            self.channels(),
            32,
        )
    }
    fn drain_decoded_at_eof(&self) -> bool {
        false
    }
    fn next_packet(&mut self) -> Result<Option<EncodedPacket>> {
        Err(crate::unsupported(
            "linked Ogg codecs emit PCM through next_step",
        ))
    }
    fn next_step(&mut self) -> Result<Option<AudioStep>> {
        self.program.next_pcm().map(|p| {
            p.map(|p| {
                AudioStep::Pcm(AudioPacket {
                    data: p.data,
                    pts: p.pts,
                    timebase_num: p.timebase_num,
                    timebase_den: p.timebase_den,
                })
            })
        })
    }
    fn rewind(&mut self) {
        self.program.rewind();
    }
    fn seek_to(&mut self, target: i64) -> i64 {
        self.program.seek_to(target)
    }
}
