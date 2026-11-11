//! Owned MP4 AAC/ALAC/PCM/ADPCM/MACE presentation decoding, including silence and repeated edits.
use crate::owned_aac::aac_ps_native::InBandPsProbe;
use crate::owned_audio_control::{AudioDecodeControl, CopyProgress};
use crate::owned_codec_config::aac_specific_config as mp4_probe_config;
pub use crate::owned_matroska_audio::{AudioDecodeStats, Error};
use crate::owned_matroska_audio::invalid;
use crate::owned_mp4::Mp4Reader as Mp4TimelineReader;
use crate::owned_mp4_audio_normalization::{
    coefficients as mp4_normalization_coefficients, opaque_positions as mp4_opaque_positions,
};
use crate::owned_mp4_audio_packet_frames::frames as mp4_coded_packet_frames;
use crate::owned_mp4_audio_schedule::AudioTimeline as Mp4AudioSchedule;
use crate::owned_mp4_clock_runs::{mp4_packet_clock_runs, mp4_tick_matches};
use crate::owned_mp4_edit_clock::{
    Segment as Mp4ExportSegment, Time as Mp4ExportTime, Timeline as Mp4ExportTimeline,
};
use fvid_control::CopyOptions;
use std::{
    io::{Read, Seek, Write},
    time::Duration,
};
type Result<T> = std::result::Result<T, Error>;

pub trait Mp4ResamplerSink: Write {
    fn output_position(&self) -> u64;
    fn take_output(&mut self) -> Vec<u8>;
    fn finish_resampling(&mut self) -> std::io::Result<u64>;
}

impl Mp4ResamplerSink for crate::owned_resample::Resampler<Vec<u8>> {
    fn output_position(&self) -> u64 {
        self.output_position()
    }
    fn take_output(&mut self) -> Vec<u8> {
        self.take_output()
    }
    fn finish_resampling(&mut self) -> std::io::Result<u64> {
        self.finish()
    }
}

impl Mp4ResamplerSink for crate::owned_resample_f64::Resampler<Vec<u8>> {
    fn output_position(&self) -> u64 {
        self.output_position()
    }
    fn take_output(&mut self) -> Vec<u8> {
        self.take_output()
    }
    fn finish_resampling(&mut self) -> std::io::Result<u64> {
        self.finish()
    }
}

pub trait Mp4SampleDecoder: Sized {
    type Sample: Copy;
    type Resampler: Mp4ResamplerSink;
    const SAMPLE_BYTES: usize;
    fn sample_to_f64(sample: Self::Sample) -> f64;
    fn push_f64_as_le(value: f64, out: &mut Vec<u8>);
    fn push_sample_le(sample: Self::Sample, out: &mut Vec<u8>);
    fn new_resampler(
        input_rate: u32,
        output_rate: u32,
        channels: u16,
        phase: (u128, u128),
    ) -> std::io::Result<Self::Resampler>;
    fn with_in_band_ps(track: &crate::owned_mp4::Track) -> Result<Self>;
    fn new(track: &crate::owned_mp4::Track) -> Result<Self>;
    fn sample_rate(&self) -> u32;
    fn channels(&self) -> u16;
    fn normalization_mask(&self) -> u32;
    fn checkpoint(&self) -> Option<Mp4AacCheckpoint>;
    fn restore_checkpoint(&mut self, state: &Mp4AacCheckpoint) -> Result<bool>;
    fn delayed(&self) -> bool;
    fn finish_delayed(&mut self) -> Result<Option<Vec<Self::Sample>>>;
    fn decode_timed(
        &mut self,
        bytes: &[u8],
        pts: u64,
        duration: u64,
    ) -> Result<Option<Vec<Self::Sample>>>;
}

pub enum Mp4AacCheckpoint {
    Aac(crate::owned_aac::AacCheckpoint),
    Ps(crate::owned_aac::aac_ps_native::Checkpoint),
}

pub enum Mp4TimelineDecoder {
    Aac(crate::owned_aac::NativeAacDecoder),
    Ps(Box<crate::owned_aac::aac_ps_native::NativePsAacDecoder>),
    Alac(crate::owned_alac::AlacDecoder),
    Pcm(crate::owned_pcm_decoder::PcmDecoder),
    Ima4(crate::owned_ima4::Ima4Decoder),
    Mace(crate::owned_mace::MaceDecoder),
    ImaWav(crate::owned_ima_wav::ImaWavDecoder),
    MsAdpcm(crate::owned_ms_adpcm::MsAdpcmDecoder),
}

impl Mp4TimelineDecoder {
    pub fn with_in_band_ps(track: &crate::owned_mp4::Track) -> Result<Self> {
        Ok(Self::Ps(Box::new(
            crate::owned_aac::aac_ps_native::NativePsAacDecoder::new_with_in_band_ps(
                mp4_probe_config(&track.configuration)?,
                track.sample_rate,
            )?,
        )))
    }

    pub fn checkpoint(&self) -> Option<Mp4AacCheckpoint> {
        match self {
            Self::Aac(d) => Some(Mp4AacCheckpoint::Aac(d.checkpoint())),
            Self::Ps(d) => Some(Mp4AacCheckpoint::Ps(d.checkpoint())),
            _ => None,
        }
    }

    pub fn restore_checkpoint(&mut self, state: &Mp4AacCheckpoint) -> Result<bool> {
        match (self, state) {
            (Self::Aac(d), Mp4AacCheckpoint::Aac(s)) => {
                d.restore(s)?;
                Ok(true)
            }
            (Self::Ps(d), Mp4AacCheckpoint::Ps(s)) => {
                d.restore(s)?;
                Ok(true)
            }
            _ => Ok(false),
        }
    }

    pub fn new(track: &crate::owned_mp4::Track) -> Result<Self> {
        match &track.codec {
            b"mp4a" => {
                let asc = crate::owned_codec_config::aac_specific_config(&track.configuration)?;
                if crate::owned_aac::config::AudioSpecificConfig::parse(asc)?.ps_present
                    == Some(true)
                {
                    Ok(Self::Ps(Box::new(
                        crate::owned_aac::aac_ps_native::NativePsAacDecoder::new(asc)?,
                    )))
                } else {
                    Ok(Self::Aac(
                        crate::owned_aac::NativeAacDecoder::new_with_output_rate(
                            asc,
                            track.sample_rate,
                        )?,
                    ))
                }
            }
            b"alac" => Ok(Self::Alac(crate::owned_alac::AlacDecoder::new(
                &track.configuration,
                track.sample_rate,
                track.channels,
            )?)),
            b"alaw" | b"ulaw" | b"raw " | b"sowt" | b"twos" | b"in24" | b"in32" | b"fl32"
            | b"fl64" => Ok(Self::Pcm(
                crate::owned_pcm_decoder::PcmDecoder::from_mp4(track)
                    .map_err(|e| invalid(&e.to_string()))?,
            )),
            b"MAC3" | b"MAC6" => Ok(Self::Mace(
                crate::owned_mace::MaceDecoder::new(
                    if track.codec == *b"MAC3" {
                        crate::owned_mace::MaceCoding::ThreeToOne
                    } else {
                        crate::owned_mace::MaceCoding::SixToOne
                    },
                    track.sample_rate,
                    track.channels,
                )
                .map_err(|e| invalid(&e.to_string()))?,
            )),
            b"ima4" => Ok(Self::Ima4(
                crate::owned_ima4::Ima4Decoder::new(track.sample_rate, track.channels)
                    .map_err(|e| invalid(&e.to_string()))?,
            )),
            b"ms\x00\x11" => Ok(Self::ImaWav(
                crate::owned_ima_wav::ImaWavDecoder::new(
                    &track.configuration,
                    track.sample_rate,
                    track.channels,
                )
                .map_err(|e| invalid(&e.to_string()))?,
            )),
            b"ms\x00\x02" => Ok(Self::MsAdpcm(
                crate::owned_ms_adpcm::MsAdpcmDecoder::new(
                    &track.configuration,
                    track.sample_rate,
                    track.channels,
                )
                .map_err(|e| invalid(&e.to_string()))?,
            )),
            _ => Err(invalid(
                "selected MP4 audio codec is not owned by this export path",
            )),
        }
    }

    pub fn sample_rate(&self) -> u32 {
        match self {
            Self::Aac(d) => d.sample_rate(),
            Self::Ps(d) => d.sample_rate(),
            Self::Alac(d) => d.sample_rate(),
            Self::Pcm(d) => d.sample_rate(),
            Self::Ima4(d) => d.sample_rate(),
            Self::Mace(d) => d.sample_rate(),
            Self::ImaWav(d) => d.sample_rate(),
            Self::MsAdpcm(d) => d.sample_rate(),
        }
    }

    pub fn channels(&self) -> u16 {
        match self {
            Self::Aac(d) => u16::from(d.channels()),
            Self::Ps(d) => u16::from(d.channels()),
            Self::Alac(d) => d.channels(),
            Self::Pcm(d) => d.channels(),
            Self::Ima4(d) => d.channels(),
            Self::Mace(d) => d.channels(),
            Self::ImaWav(d) => d.channels(),
            Self::MsAdpcm(d) => d.channels(),
        }
    }

    pub fn channel_mask_opt(&self) -> Option<u32> {
        match self {
            Self::Aac(d) => Some(d.channel_mask()),
            Self::Ps(d) => Some(d.channel_mask()),
            Self::Alac(d) => Some(d.channel_mask()),
            _ => None,
        }
    }

    pub fn channel_mask(&self) -> u32 {
        self.channel_mask_opt().unwrap_or_else(|| {
            crate::owned_pcm_channels::standard_mask(self.channels()).unwrap_or(0) as u32
        })
    }

    pub fn normalization_mask(&self) -> u32 {
        self.channel_mask()
    }

    pub fn delayed(&self) -> bool {
        matches!(self, Self::Ps(_)) || matches!(self, Self::Aac(d) if d.delayed())
    }

    pub fn finish_delayed(&mut self) -> Result<Option<Vec<f32>>> {
        match self {
            Self::Ps(d) => Ok(d.finish()?.map(|f| f.pcm)),
            Self::Aac(d) => Ok(d.finish()?.map(|f| f.samples)),
            _ => Ok(None),
        }
    }

    pub fn decode_timed(
        &mut self,
        bytes: &[u8],
        pts: u64,
        duration: u64,
    ) -> Result<Option<Vec<f32>>> {
        let pcm = match self {
            Self::Ps(d) => return Ok(d.decode(bytes)?.map(|f| f.pcm)),
            Self::Aac(d) => {
                return Ok(d
                    .decode_timed(
                        bytes,
                        i64::try_from(pts).map_err(|_| invalid("AAC timestamp overflow"))?,
                        duration,
                    )?
                    .map(|f| f.samples));
            }
            Self::Alac(d) => d.decode_pcm(bytes)?,
            Self::Pcm(d) => d.decode_pcm(bytes).map_err(|e| invalid(&e.to_string()))?,
            Self::Ima4(d) => d.decode_pcm(bytes).map_err(|e| invalid(&e.to_string()))?,
            Self::Mace(d) => d.decode_pcm(bytes).map_err(|e| invalid(&e.to_string()))?,
            Self::ImaWav(d) => d.decode_pcm(bytes).map_err(|e| invalid(&e.to_string()))?,
            Self::MsAdpcm(d) => d.decode_pcm(bytes).map_err(|e| invalid(&e.to_string()))?,
        };
        Ok(Some(pcm))
    }
}

impl Mp4SampleDecoder for Mp4TimelineDecoder {
    type Sample = f32;
    type Resampler = crate::owned_resample::Resampler<Vec<u8>>;
    const SAMPLE_BYTES: usize = 4;

    fn sample_to_f64(sample: f32) -> f64 {
        f64::from(sample)
    }
    fn push_f64_as_le(value: f64, out: &mut Vec<u8>) {
        out.extend_from_slice(&(value as f32).to_le_bytes());
    }
    fn push_sample_le(sample: f32, out: &mut Vec<u8>) {
        out.extend_from_slice(&sample.to_le_bytes());
    }
    fn new_resampler(
        input_rate: u32,
        output_rate: u32,
        channels: u16,
        phase: (u128, u128),
    ) -> std::io::Result<Self::Resampler> {
        crate::owned_resample::Resampler::new_with_origin(
            Vec::new(),
            input_rate,
            output_rate,
            channels,
            0,
            phase,
        )
    }
    fn with_in_band_ps(track: &crate::owned_mp4::Track) -> Result<Self> {
        Self::with_in_band_ps(track)
    }
    fn new(track: &crate::owned_mp4::Track) -> Result<Self> {
        Self::new(track)
    }
    fn sample_rate(&self) -> u32 {
        self.sample_rate()
    }
    fn channels(&self) -> u16 {
        self.channels()
    }
    fn normalization_mask(&self) -> u32 {
        self.normalization_mask()
    }
    fn checkpoint(&self) -> Option<Mp4AacCheckpoint> {
        self.checkpoint()
    }
    fn restore_checkpoint(&mut self, state: &Mp4AacCheckpoint) -> Result<bool> {
        self.restore_checkpoint(state)
    }
    fn delayed(&self) -> bool {
        self.delayed()
    }
    fn finish_delayed(&mut self) -> Result<Option<Vec<f32>>> {
        self.finish_delayed()
    }
    fn decode_timed(
        &mut self,
        bytes: &[u8],
        pts: u64,
        duration: u64,
    ) -> Result<Option<Vec<f32>>> {
        self.decode_timed(bytes, pts, duration)
    }
}

pub fn mp4_audio_index<R: Read + Seek>(
    reader: &Mp4TimelineReader<R>,
    selected: Option<usize>,
) -> Result<usize> {
    let indices: Vec<_> = if selected.is_none() {
        reader.automatic_track_indices(&[*b"soun"])
    } else {
        reader
            .tracks()
            .iter()
            .enumerate()
            .filter_map(|(index, t)| (t.handler == *b"soun").then_some(index))
            .collect()
    };
    match selected {
        Some(index) if indices.contains(&index) => Ok(index),
        None if indices.len() == 1 => Ok(indices[0]),
        _ => Err(invalid("select exactly one audio stream")),
    }
}

fn mp4_decoder_admission(track: &crate::owned_mp4::Track) -> Result<usize> {
    Ok(match &track.codec {
        b"mp4a" => {
            // One decoder state, including SBR state and DSP.
            crate::owned_aac::stream::decode_config_admission_bytes(
                crate::owned_codec_config::aac_specific_config(&track.configuration)?,
                track.sample_rate,
            )?
        }
        b"alac" => crate::owned_alac::AlacDecoder::decode_admission_bytes(
            &track.configuration,
            track.sample_rate,
            track.channels,
        )
        .map_err(|e| invalid(&e.to_string()))?,
        b"alaw" | b"ulaw" | b"raw " | b"sowt" | b"twos" | b"in24" | b"in32" | b"fl32"
        | b"fl64" => {
            crate::owned_pcm_decoder::PcmDecoder::from_mp4(track)
                .map_err(|e| invalid(&e.to_string()))?;
            16 * 1024
        }
        b"MAC3" | b"MAC6" => {
            crate::owned_mace::MaceDecoder::new(
                if track.codec == *b"MAC3" {
                    crate::owned_mace::MaceCoding::ThreeToOne
                } else {
                    crate::owned_mace::MaceCoding::SixToOne
                },
                track.sample_rate,
                track.channels,
            )
            .map_err(|e| invalid(&e.to_string()))?;
            16 * 1024
        }
        b"ima4" => {
            crate::owned_ima4::Ima4Decoder::new(track.sample_rate, track.channels)
                .map_err(|e| invalid(&e.to_string()))?;
            16 * 1024
        }
        b"ms\x00\x11" => {
            crate::owned_ima_wav::ImaWavDecoder::new(
                &track.configuration,
                track.sample_rate,
                track.channels,
            )
            .map_err(|e| invalid(&e.to_string()))?;
            16 * 1024
        }
        b"ms\x00\x02" => {
            crate::owned_ms_adpcm::MsAdpcmDecoder::new(
                &track.configuration,
                track.sample_rate,
                track.channels,
            )
            .map_err(|e| invalid(&e.to_string()))?;
            16 * 1024
        }
        _ => {
            return Err(invalid(
                "selected MP4 audio codec is not owned by this export path",
            ));
        }
    })
}

/// Admit retained MP4 AAC/ALAC/PCM/ADPCM/MACE decode payload before cloning the track or creating
/// decoder/checkpoint state. Container parsing has its own bounded limits;
/// parser temporaries and caller-owned I/O are outside this retained estimate.
#[allow(dead_code)]
pub(crate) fn admit_audio_reader<R: Read + Seek>(
    reader: &Mp4TimelineReader<R>,
    index: usize,
    options: &CopyOptions,
) -> Result<()> {
    admit_audio_reader_with_interval(reader, index, None, options, None)
}

pub(crate) fn admit_audio_reader_with_interval<R: Read + Seek>(
    reader: &Mp4TimelineReader<R>,
    index: usize,
    interval: Option<(Duration, Duration)>,
    options: &CopyOptions,
    target: Option<(u32, u16, u32)>,
) -> Result<()> {
    if options
        .cancel
        .as_ref()
        .is_some_and(fvid_control::CancelFlag::is_cancelled)
    {
        return Err(invalid("media operation cancelled"));
    }
    if options.max_packet_bytes == 0 {
        return Err(invalid("packet byte limit must be positive"));
    }
    if options.max_packets == Some(0) {
        return Err(invalid("packet work limit requires at least one packet"));
    }
    if options.max_controlled_bytes == Some(0) {
        return Err(invalid("controlled memory limit must be positive"));
    }
    crate::owned_budget::check_rss_budget(options).map_err(|e| invalid(&e))?;
    let track = reader
        .tracks()
        .get(index)
        .ok_or_else(|| invalid("selected audio stream is absent"))?;
    let active_samples =
        mp4_active_samples(track, reader.movie_timescale(), interval, options.max_packets);
    let largest_packet = match &track.samples {
        crate::owned_mp4::SampleIndex::Expanded(samples) => samples
            .iter()
            .take(active_samples)
            .map(|s| s.size as usize)
            .max()
            .unwrap_or(0),
        crate::owned_mp4::SampleIndex::Uniform(index) => {
            if active_samples == 0 {
                0
            } else {
                index.bytes_per_frame as usize
            }
        }
    };
    if largest_packet > options.max_packet_bytes {
        return Err(invalid("MP4 packet exceeds budget"));
    }
    let Some(limit) = options.max_controlled_bytes else {
        return Ok(());
    };
    let mut decoder = 0usize;
    let mut checkpoint = 0usize;
    let mut configuration = 0usize;
    let mut expansion = 2usize;
    let initial_description = track.samples.get(0).map(|s| s.description_index);
    for i in 0..track.sample_descriptions.len() {
        // Decoder initialization precedes scheduling even for a silence-only edit.
        if initial_description != Some((i + 1) as u32)
            && !track.description_used_before((i + 1) as u32, active_samples)
        {
            continue;
        }
        let setup = track.decoder_setup((i + 1) as u32)?;
        let state = mp4_decoder_admission(&setup)?;
        // Charge decoded-packet growth only for admitted decoder setups.
        // Dormant catalog descriptions are retained metadata, not PCM buffers.
        expansion = expansion.max(match &setup.codec {
            b"MAC3" | b"MAC6" => 64,
            b"alaw" | b"ulaw" | b"raw " | b"sowt" | b"twos" | b"in24" | b"in32" | b"fl32"
            | b"fl64" | b"ima4" | b"ms\x00\x11" | b"ms\x00\x02" => 10,
            _ => 2,
        });
        decoder = decoder.max(state);
        if setup.codec == *b"mp4a" {
            checkpoint = checkpoint.max(state);
        }
        configuration = configuration.max(setup.configuration.len());
    }
    // Active plus replacement/restore state and one retained AAC checkpoint.
    // For constant AAC this remains the previous three-state admission bound.
    let decoder = decoder
        .checked_mul(2)
        .and_then(|n| n.checked_add(checkpoint))
        .ok_or_else(|| invalid("MP4 audio memory estimate overflow"))?;
    let overflow = || invalid("MP4 audio memory estimate overflow");
    let mut estimated = reader.estimated_index_payload_bytes()?;
    let mut add = |bytes: usize| -> Result<()> {
        estimated = estimated.checked_add(bytes).ok_or_else(overflow)?;
        Ok(())
    };
    add(decoder)?;
    add(track.sample_descriptions.len())?; // per-description in-band PS flags
    add(configuration.checked_mul(2).ok_or_else(overflow)?)?; // initial and replacement decoder headers
    if target.is_some()
        || track.has_declared_gaps()
        || track.description_for_sample(0).is_some_and(|d| {
            d.sample_rate > 0
                && (track.timescale % d.sample_rate != 0
                    || (track.edits.is_empty()
                        && track.timescale > 0
                        && (u128::from(track.duration) * u128::from(d.sample_rate))
                            % u128::from(track.timescale)
                            != 0))
        })
        || track.description_changes_before(active_samples)
    {
        let first = track
            .description_for_sample(0)
            .ok_or_else(|| invalid("MP4 audio has no samples"))?;
        let output_rate = target.map_or(first.sample_rate, |t| t.0);
        let output_channels = usize::from(target.map_or(first.channels.max(2), |t| t.1));
        let mut normalization = 0usize;
        for (i, description) in track.sample_descriptions.iter().enumerate() {
            if !track.description_used_before((i + 1) as u32, active_samples) {
                continue;
            }
            if description.sample_rate == 0 || output_rate == 0 {
                return Err(invalid("zero MP4 normalization clock"));
            }
            let radius = usize::try_from(crate::owned_resample::input_radius(
                description.sample_rate,
                output_rate,
            ))
            .map_err(|_| overflow())?;
            // Geometric queue capacity, f64 slots (also bounds f32), four scalar
            // work arrays, mixed input, and a drained output block coexist.
            let ring = radius
                .checked_mul(4)
                .and_then(|n| n.checked_add(16))
                .and_then(|n| n.checked_mul(255 * 8))
                .ok_or_else(overflow)?;
            let frames =
                usize::try_from(u64::from(output_rate).div_ceil(u64::from(description.sample_rate)))
                    .map_err(|_| overflow())?
                    .max(4096);
            let buffers = frames
                .checked_mul(output_channels)
                .and_then(|n| n.checked_mul(8 * 4))
                .and_then(|n| n.checked_add(4 * 255 * 8 + 256 * 8 * output_channels))
                .ok_or_else(overflow)?;
            normalization = normalization.max(ring.checked_add(buffers).ok_or_else(overflow)?);
        }
        add(normalization)?;
        // Two geometric capacities of rational per-configuration clock records.
        let clock_records = if track.has_declared_gaps() {
            track.samples.len()
        } else {
            active_samples
        };
        add(clock_records.max(4).checked_mul(320).ok_or_else(overflow)?)?;
    }

    for bytes in [
        track.name.len(),
        track.language.len(),
        track.configuration.len(),
    ] {
        add(bytes)?;
    }
    add(track
        .sample_descriptions
        .len()
        .checked_mul(std::mem::size_of::<crate::owned_mp4::SampleDescription>())
        .ok_or_else(overflow)?)?;
    for description in &track.sample_descriptions {
        add(description.configuration.len())?;
    }
    add(track
        .edits
        .len()
        .checked_mul(std::mem::size_of::<crate::owned_mp4::Edit>())
        .ok_or_else(overflow)?)?;
    add(track
        .edits
        .len()
        .max(1)
        .checked_mul(std::mem::size_of::<crate::owned_mp4_audio_schedule::Segment>())
        .ok_or_else(overflow)?)?;
    let (records, width) = match &track.samples {
        crate::owned_mp4::SampleIndex::Expanded(samples) => {
            (samples.len(), std::mem::size_of::<crate::owned_mp4::Sample>())
        }
        crate::owned_mp4::SampleIndex::Uniform(index) => (
            index.runs.len(),
            std::mem::size_of::<crate::owned_mp4::FrameRun>(),
        ),
    };
    add(records.checked_mul(width).ok_or_else(overflow)?)?;
    // The reusable packet Vec may grow geometrically while reading larger packets.
    add(largest_packet.checked_mul(expansion).ok_or_else(overflow)?)?;
    if !options.metadata_set.is_empty() || !options.metadata_delete.is_empty() {
        let info = crate::owned_wave_metadata::info_for_file_tags(
            reader.tags(),
            &track.name,
            &track.language,
            Some(reader.encoder()),
            &std::collections::BTreeMap::new(),
        )
        .map_err(|e| invalid(&e))?;
        let wave_meta_bytes =
            crate::owned_wave_metadata::wave_metadata_working_bytes(info.len(), options)
                .map_err(|e| invalid(&e))?;
        add(wave_meta_bytes)?;
    }
    if estimated > limit {
        return Err(invalid(&format!(
            "controlled memory budget exceeded: need {estimated} bytes, limit {limit}"
        )));
    }
    Ok(())
}

/// Decode normalized float32 PCM. Errors may leave partial caller-owned output;
/// progress never reports publication. Retained allocation admission is checked
/// before decoding; metadata edits remain unsupported.
/// Encoded packet limits include required preroll.
pub fn decode_mp4_audio_pcm<R: Read + Seek>(
    source: R,
    output: &mut impl Write,
    interval: Option<(Duration, Duration)>,
    options: &CopyOptions,
) -> Result<AudioDecodeStats> {
    decode_mp4_audio_pcm_targeted(source, output, interval, options, None)
}

pub(crate) fn decode_mp4_audio_pcm_targeted<R: Read + Seek>(
    source: R,
    output: &mut impl Write,
    interval: Option<(Duration, Duration)>,
    options: &CopyOptions,
    target: Option<(u32, u16, u32)>,
) -> Result<AudioDecodeStats> {
    let mut control = CopyProgress::new(options);
    control.check()?;
    if !options.metadata_set.is_empty()
        || !options.metadata_delete.is_empty()
        || !options.stream_metadata_set.is_empty()
        || !options.stream_metadata_delete.is_empty()
    {
        return Err(invalid("raw PCM stream cannot apply metadata mutations"));
    }
    let selected = match options.streams.as_slice() {
        [] => None,
        [index] => Some(*index),
        _ => return Err(invalid("select exactly one audio stream")),
    };
    if let Some(hook) = &options.progress {
        hook.emit(control.event);
    }
    control.check()?;
    let metadata_bytes = crate::owned_mp4::Limits::default()
        .metadata_bytes
        .min(options.max_controlled_bytes.unwrap_or(usize::MAX));
    let mut reader = Mp4TimelineReader::open_indexed(
        source,
        crate::owned_mp4::Limits {
            metadata_bytes,
            ..Default::default()
        },
    )
    .map_err(|error| {
        if metadata_bytes < crate::owned_mp4::Limits::default().metadata_bytes
            && error.to_string().contains("MP4 metadata exceeds budget")
        {
            invalid("controlled memory budget exceeded: MP4 metadata exceeds budget")
        } else {
            error.into()
        }
    })?;
    reader.restrict_packet_bytes(options.max_packet_bytes);
    let index = mp4_audio_index(&reader, selected)?;
    admit_audio_reader_with_interval(&reader, index, interval, options, target)?;
    decode_mp4_audio_reader_targeted(reader, output, interval, selected, &mut control, target)
}

pub(crate) fn decode_mp4_audio_reader_pcm_targeted<R: Read + Seek>(
    reader: Mp4TimelineReader<R>,
    output: &mut impl Write,
    interval: Option<(Duration, Duration)>,
    options: &CopyOptions,
    target: Option<(u32, u16, u32)>,
) -> Result<AudioDecodeStats> {
    let mut control = CopyProgress::new(options);
    control.check()?;
    if !options.metadata_set.is_empty()
        || !options.metadata_delete.is_empty()
        || !options.stream_metadata_set.is_empty()
        || !options.stream_metadata_delete.is_empty()
    {
        return Err(invalid("raw PCM stream cannot apply metadata mutations"));
    }
    let selected = match options.streams.as_slice() {
        [] => None,
        [index] => Some(*index),
        _ => return Err(invalid("select exactly one audio stream")),
    };
    if let Some(hook) = &options.progress {
        hook.emit(control.event);
    }
    control.check()?;
    decode_mp4_audio_reader_targeted(reader, output, interval, selected, &mut control, target)
}

fn mp4_write_tail_silence<C: AudioDecodeControl>(
    output: &mut impl Write,
    frames: u64,
    channels: u16,
    sample_bytes: usize,
    control: &C,
) -> Result<()> {
    let mut bytes = frames
        .checked_mul(u64::from(channels) * sample_bytes as u64)
        .ok_or_else(|| invalid("MP4 tail silence size overflow"))?;
    let zeros = [0u8; 4096];
    while bytes != 0 {
        control.check()?;
        let count = bytes.min(zeros.len() as u64) as usize;
        output.write_all(&zeros[..count])?;
        bytes -= count as u64;
    }
    Ok(())
}

pub fn mp4_active_samples(
    track: &crate::owned_mp4::Track,
    movie_scale: u32,
    interval: Option<(Duration, Duration)>,
    max_packets: Option<u64>,
) -> usize {
    let edit_prefix = mp4_immediate_edit_prefix(track, movie_scale, interval);
    let media_edits = track.edits.iter().filter(|e| e.media_time >= 0).count();
    if media_edits > 1 && edit_prefix.is_none() {
        return track.samples.len();
    }
    let mut active = max_packets
        .map(|m| usize::try_from(m).unwrap_or(usize::MAX))
        .unwrap_or(track.samples.len())
        .min(track.samples.len());
    if let Some((_, end)) = interval
        && track.timescale > 0
        && (track.edits.is_empty() || (track.edits.len() == 1 && track.edits[0].media_time == 0))
    {
        if end.is_zero() {
            return 0;
        }
        let scale = u128::from(track.timescale);
        if let Some(end_scaled) = end.as_nanos().checked_mul(scale) {
            let mut interval_samples = 0usize;
            while interval_samples < active {
                let Some(sample) = track.samples.get(interval_samples) else {
                    break;
                };
                if sample.pts >= 0
                    && (sample.pts as u128).saturating_mul(1_000_000_000) >= end_scaled
                {
                    break;
                }
                interval_samples += 1;
            }
            active = active.min(interval_samples);
        }
    }
    active.min(edit_prefix.unwrap_or(active))
}

// Immediate, unchanged decoder geometry uses the exact integer presentation
// schedule below. Keep delayed AAC and normalized clocks conservative: their
// entropy probing/lookahead can legitimately read beyond an audible endpoint.
fn mp4_immediate_edit_prefix(
    track: &crate::owned_mp4::Track,
    movie_scale: u32,
    interval: Option<(Duration, Duration)>,
) -> Option<usize> {
    if track.edits.is_empty() || track.has_declared_gaps() {
        return None;
    }
    let first = track.description_for_sample(0)?;
    // A dormant catalog entry cannot extend the work admitted for an edit.
    // Keep the catalog-only shortcut for ordinary identical descriptions;
    // otherwise compare referenced headers with compact-index-aware traversal.
    if first.sample_rate == 0
        || track.timescale % first.sample_rate != 0
        || (!track.sample_descriptions.iter().all(|d| d == first)
            && track.description_changes_before(track.samples.len()))
        || !matches!(
            &first.codec,
            b"alaw"
                | b"ulaw"
                | b"raw "
                | b"sowt"
                | b"twos"
                | b"in24"
                | b"in32"
                | b"fl32"
                | b"fl64"
                | b"alac"
                | b"ima4"
                | b"MAC3"
                | b"MAC6"
                | b"ms\x00\x11"
                | b"ms\x00\x02"
        )
    {
        return None;
    }
    let rate = first.sample_rate;
    let clock = track.resolved_edit_clock(movie_scale).ok()?;
    let schedule = Mp4AudioSchedule::new(
        clock.edits(),
        track.duration,
        track.timescale,
        clock.timescale,
        rate,
    )
    .ok()?;
    let boundary = |t: Duration| -> Option<u64> {
        u64::try_from(
            t.as_nanos()
                .checked_mul(u128::from(rate))?
                .div_ceil(1_000_000_000),
        )
        .ok()
    };
    let (from, to) = match interval {
        Some((a, b)) => (boundary(a)?, boundary(b)?.min(schedule.sample_frames)),
        None => (0, schedule.sample_frames),
    };
    let mut last = 0u64;
    for segment in schedule.segments {
        let begin = from.max(segment.presentation.start);
        let end = to.min(segment.presentation.end);
        if begin < end
            && let Some(source) = segment.source_start
        {
            last = last.max(source.checked_add(end - segment.presentation.start)?);
        }
    }
    let ticks_per_frame = u64::from(track.timescale / rate);
    if let crate::owned_mp4::SampleIndex::Uniform(index) = &track.samples {
        if index.duration == 0 || u64::from(index.duration) % ticks_per_frame != 0 {
            return None;
        }
        let ticks = u128::from(last).checked_mul(u128::from(ticks_per_frame))?;
        return Some(
            usize::try_from(ticks.div_ceil(u128::from(index.duration)))
                .ok()?
                .min(index.count),
        );
    }
    let mut count = 0;
    while count < track.samples.len() {
        let sample = track.samples.get(count)?;
        let pts = u64::try_from(sample.pts).ok()?;
        if pts % ticks_per_frame != 0 {
            return None;
        }
        if pts / ticks_per_frame >= last {
            break;
        }
        count += 1;
    }
    Some(count)
}

pub fn decode_mp4_audio_reader_controlled<R: Read + Seek, C: AudioDecodeControl>(
    reader: Mp4TimelineReader<R>,
    output: &mut impl Write,
    interval: Option<(Duration, Duration)>,
    selected: Option<usize>,
    control: &mut C,
) -> Result<AudioDecodeStats> {
    decode_mp4_audio_reader_targeted(reader, output, interval, selected, control, None)
}

pub fn decode_mp4_audio_reader_targeted<R: Read + Seek, C: AudioDecodeControl>(
    reader: Mp4TimelineReader<R>,
    output: &mut impl Write,
    interval: Option<(Duration, Duration)>,
    selected: Option<usize>,
    control: &mut C,
    target: Option<(u32, u16, u32)>,
) -> Result<AudioDecodeStats> {
    decode_mp4_audio_reader_generic::<R, C, Mp4TimelineDecoder>(
        reader, output, interval, selected, control, target,
    )
}

pub fn decode_mp4_audio_reader_generic<
    R: Read + Seek,
    C: AudioDecodeControl,
    D: Mp4SampleDecoder,
>(
    mut reader: Mp4TimelineReader<R>,
    output: &mut impl Write,
    interval: Option<(Duration, Duration)>,
    selected: Option<usize>,
    control: &mut C,
    target: Option<(u32, u16, u32)>,
) -> Result<AudioDecodeStats> {
    control.check()?;
    if interval.is_some_and(|(from, to)| from >= to) {
        return Err(invalid("audio interval requires from < to"));
    }
    let index = mp4_audio_index(&reader, selected)?;
    let track = reader.tracks()[index].clone();
    let initial_description = track
        .samples
        .get(0)
        .ok_or_else(|| invalid("MP4 audio has no samples"))?
        .description_index;
    let initial_setup = track.decoder_setup(initial_description)?;
    let max_packets = control.packet_limit();
    let active_samples =
        mp4_active_samples(&track, reader.movie_timescale(), interval, max_packets);
    let mut ps = vec![false; track.sample_descriptions.len()];
    // Probe each used configuration independently; never feed packets from a
    // different setup into an AAC entropy/history state.
    for (i, flag) in ps.iter_mut().enumerate() {
        if track.description_used_before((i + 1) as u32, active_samples) {
            *flag = negotiate_mp4_description_ps_with_interval(
                &mut reader,
                index,
                (i + 1) as u32,
                interval,
                max_packets,
                || control.check(),
            )?;
        }
    }
    let make_decoder = |id: u32| -> Result<D> {
        let setup = track.decoder_setup(id)?;
        if ps[(id - 1) as usize] {
            D::with_in_band_ps(&setup)
        } else {
            D::new(&setup)
        }
    };
    let mut decoder = make_decoder(initial_description)?;
    let rate = decoder.sample_rate();
    let channels = decoder.channels();
    if track.timescale == 0
        || initial_setup.sample_rate != rate
        || (!ps[(initial_description - 1) as usize] && initial_setup.channels != channels)
    {
        return Err(invalid(
            "MP4 audio export requires valid clock and matching audio geometry",
        ));
    }
    // Detect the need for normalization before caller-owned output.
    let mut checked = initial_description;
    // Even a prefix with one decoder setup can belong to a longer track whose
    // endpoint is fractional on that setup's grid. The integer schedule would
    // reject the whole duration before reaching the requested short interval.
    let fractional_extent = track.edits.is_empty()
        && (u128::from(track.duration) * u128::from(rate)) % u128::from(track.timescale) != 0;
    let mut normalize = target.is_some()
        || track.timescale % rate != 0
        || fractional_extent
        || track.has_declared_gaps();
    for i in 0..active_samples {
        control.check()?;
        let id = track
            .samples
            .get(i)
            .ok_or_else(|| invalid("missing audio sample"))?
            .description_index;
        if track.sample_descriptions[(id - 1) as usize]
            != track.sample_descriptions[(checked - 1) as usize]
        {
            let next = make_decoder(id)?;
            if next.sample_rate() != rate
                || next.channels() != channels
                || next.normalization_mask() != decoder.normalization_mask()
            {
                normalize = true;
            }
            checked = id;
        }
    }
    if normalize {
        drop(initial_setup);
        return decode_mp4_audio_normalized::<R, C, D>(
            reader,
            output,
            interval,
            index,
            track,
            initial_description,
            ps,
            decoder,
            target,
            control,
        );
    }
    let sample_position = |ticks: u64| -> Result<u64> {
        let numerator = u128::from(ticks) * u128::from(rate);
        let denominator = u128::from(track.timescale);
        if numerator % denominator != 0 {
            return Err(invalid("MP4 audio timestamp is not aligned to a sample"));
        }
        u64::try_from(numerator / denominator).map_err(|_| invalid("audio sample position overflow"))
    };
    let edit_clock = track.resolved_edit_clock(reader.movie_timescale())?;
    let presentation = Mp4AudioSchedule::new(
        edit_clock.edits(),
        track.duration,
        track.timescale,
        edit_clock.timescale,
        rate,
    )?;
    let declared_tail = track
        .declared_empty_tail()?
        .map(|tail| -> Result<(u64, u64)> {
            let boundary = |ticks| -> Result<u64> {
                u64::try_from(
                    Mp4ExportTime::new(ticks, u128::from(track.timescale))?.ceil_frames(rate)?,
                )
                .map_err(|_| invalid("MP4 tail sample position overflow"))
            };
            Ok((boundary(tail.start)?, boundary(tail.end)?))
        })
        .transpose()?;
    let timeline = presentation.sample_frames;
    let segments = presentation.segments;
    let (from, to) = if let Some((begin, end)) = interval {
        let boundary = |time: Duration| -> Result<u64> {
            let value = time
                .as_nanos()
                .checked_mul(u128::from(rate))
                .ok_or_else(|| invalid("audio interval overflow"))?
                .div_ceil(1_000_000_000);
            u64::try_from(value).map_err(|_| invalid("audio interval overflow"))
        };
        (boundary(begin)?, boundary(end)?.min(timeline))
    } else {
        (0, timeline)
    };
    if from >= to {
        return Err(invalid("audio interval contains no samples"));
    }
    let mut stats = AudioDecodeStats {
        sample_frames: 0,
        decoded_frames: 0,
        sample_rate: rate,
        channels,
    };
    let mut packet = Vec::new();
    let mut pcm_scratch = Vec::new();
    pcm_scratch
        .try_reserve_exact(4096 * D::SAMPLE_BYTES)
        .map_err(|_| invalid("MP4 audio PCM output buffer allocation failed"))?;
    // One source-boundary checkpoint bounds retained state regardless of the
    // number of edits. Checkpoints also retain their active setup identity.
    let mut checkpoint: Option<(
        usize,
        u32,
        u64,
        Option<u64>,
        Mp4AacCheckpoint,
        std::collections::VecDeque<(u64, u64)>,
    )> = None;
    for segment in segments {
        control.check()?;
        if control.packet_limit_reached() {
            break;
        }
        let segment_start = segment.presentation.start;
        let segment_end = segment.presentation.end;
        let source = segment.source_start;
        let begin = from.max(segment_start);
        let end = to.min(segment_end);
        if begin >= end {
            continue;
        }
        let length = end - begin;
        let Some(source) = source else {
            let zeros = [0u8; 4096];
            let mut bytes = length
                .checked_mul(u64::from(channels) * D::SAMPLE_BYTES as u64)
                .ok_or_else(|| invalid("audio silence size overflow"))?;
            while bytes != 0 {
                control.check()?;
                let count = bytes.min(zeros.len() as u64) as usize;
                output.write_all(&zeros[..count])?;
                bytes -= count as u64;
            }
            stats.sample_frames += length;
            continue;
        };
        let from = source
            .checked_add(begin - segment_start)
            .ok_or_else(|| invalid("audio source edit overflow"))?;
        let to = from
            .checked_add(length)
            .ok_or_else(|| invalid("audio source edit overflow"))?;
        decoder = make_decoder(initial_description)?;
        let mut active_description = initial_description;
        let mut first_sample = 0;
        let mut expected = None;
        let mut pending_window = std::collections::VecDeque::new();
        if let Some((index, description, start, previous, state, pending)) = &checkpoint
            && *start <= from
        {
            decoder = make_decoder(*description)?;
            if decoder.restore_checkpoint(state)? {
                active_description = *description;
                first_sample = *index;
                expected = *previous;
                pending_window = pending.clone();
            }
        }
        let mut captured = false;
        let mut written = 0u64;
        let mut sample_index = first_sample;
        loop {
            control.check()?;
            let terminal = sample_index == track.samples.len() || control.packet_limit_reached();
            let changing = !terminal && {
                let id = track
                    .samples
                    .get(sample_index)
                    .ok_or_else(|| invalid("missing audio sample"))?
                    .description_index;
                track.sample_descriptions[(id - 1) as usize]
                    != track.sample_descriptions[(active_description - 1) as usize]
            };
            let (start, duration, samples) = if terminal || (changing && !pending_window.is_empty())
            {
                let Some(samples) = decoder.finish_delayed()? else {
                    if !pending_window.is_empty() {
                        return Err(invalid("delayed MP4 audio left an undrained source window"));
                    }
                    break;
                };
                let (start, duration) = pending_window
                    .pop_front()
                    .ok_or_else(|| invalid("delayed MP4 audio has no source window"))?;
                (start, duration, samples)
            } else {
                let sample = track
                    .samples
                    .get(sample_index)
                    .ok_or_else(|| invalid("missing audio sample"))?;
                if changing {
                    decoder = make_decoder(sample.description_index)?;
                    active_description = sample.description_index;
                }
                let start = sample_position(
                    u64::try_from(sample.pts).map_err(|_| invalid("negative audio timestamp"))?,
                )?;
                let duration = sample_position(u64::from(sample.duration))?;
                if expected.is_some_and(|value| value != start) {
                    return Err(invalid("non-contiguous MP4 audio timeline"));
                }
                if start >= to
                    && (!decoder.delayed()
                        || pending_window.front().is_none_or(|(at, _)| *at >= to))
                {
                    break;
                }
                if !captured
                    && start
                        .checked_add(duration)
                        .ok_or_else(|| invalid("audio timestamp overflow"))?
                        > from
                {
                    if let Some(state) = decoder.checkpoint() {
                        checkpoint = Some((
                            sample_index,
                            active_description,
                            start,
                            expected,
                            state,
                            pending_window.clone(),
                        ));
                    }
                    captured = true;
                }
                expected = Some(
                    start
                        .checked_add(duration)
                        .ok_or_else(|| invalid("audio timestamp overflow"))?,
                );
                reader.read_packet(index, sample_index, &mut packet)?;
                let samples = decoder.decode_timed(&packet, start, duration)?;
                control.packet(packet.len())?;
                sample_index += 1;
                if decoder.delayed() {
                    pending_window.push_back((start, duration));
                    if pending_window.len() > 3 {
                        return Err(invalid("delayed MP4 source lookahead exceeded"));
                    }
                    let Some(samples) = samples else {
                        continue;
                    };
                    let (start, duration) = pending_window
                        .pop_front()
                        .ok_or_else(|| invalid("delayed MP4 source identity mismatch"))?;
                    (start, duration, samples)
                } else {
                    (
                        start,
                        duration,
                        samples.ok_or_else(|| invalid("immediate MP4 audio returned no PCM"))?,
                    )
                }
            };
            if !samples.len().is_multiple_of(usize::from(channels)) {
                return Err(invalid("invalid MP4 decoded PCM stride"));
            }
            let frames = (samples.len() / usize::from(channels)) as u64;
            if duration == 0 || duration > frames {
                return Err(invalid(
                    "MP4 audio packet duration disagrees with decoded samples",
                ));
            }
            // The container assigns a presentation window to every packet.
            // Short windows trim decoded padding even in the interior; decoder
            // overlap state still consumes the complete compressed packet.
            let first = from.saturating_sub(start).min(duration) as usize;
            let last = to.saturating_sub(start).min(duration) as usize;
            for chunk in samples
                [first * usize::from(channels)..last.max(first) * usize::from(channels)]
                .chunks(4096)
            {
                pcm_scratch.clear();
                for &value in chunk {
                    D::push_sample_le(value, &mut pcm_scratch);
                }
                output.write_all(&pcm_scratch)?;
            }
            written += last.saturating_sub(first) as u64;
            stats.decoded_frames += 1;
            if written == length {
                break;
            }
        }
        if written != length && !control.packet_limit_reached() {
            let position = from
                .checked_add(written)
                .ok_or_else(|| invalid("MP4 tail position overflow"))?;
            if sample_index == track.samples.len()
                && declared_tail.is_some_and(|(start, end)| position >= start && to <= end)
            {
                mp4_write_tail_silence(
                    output,
                    length - written,
                    channels,
                    D::SAMPLE_BYTES,
                    control,
                )?;
                written = length;
            } else {
                return Err(invalid("audio edit extends outside available samples"));
            }
        }
        stats.sample_frames += written;
    }
    if stats.sample_frames == 0 {
        return Err(invalid("MP4 audio edit contains no samples"));
    }
    Ok(stats)
}

pub fn negotiate_mp4_ps<R: Read + Seek>(
    reader: &mut Mp4TimelineReader<R>,
    index: usize,
    max_packets: Option<u64>,
    check: impl FnMut() -> Result<()>,
) -> Result<bool> {
    negotiate_mp4_ps_with_interval(reader, index, None, max_packets, check)
}

pub fn negotiate_mp4_ps_with_interval<R: Read + Seek>(
    reader: &mut Mp4TimelineReader<R>,
    index: usize,
    interval: Option<(Duration, Duration)>,
    max_packets: Option<u64>,
    check: impl FnMut() -> Result<()>,
) -> Result<bool> {
    let Some(first) = reader.tracks()[index].samples.get(0) else {
        return Ok(false);
    };
    negotiate_mp4_description_ps_with_interval(
        reader,
        index,
        first.description_index,
        interval,
        max_packets,
        check,
    )
}

fn negotiate_mp4_description_ps_with_interval<R: Read + Seek>(
    reader: &mut Mp4TimelineReader<R>,
    index: usize,
    description: u32,
    interval: Option<(Duration, Duration)>,
    max_packets: Option<u64>,
    mut check: impl FnMut() -> Result<()>,
) -> Result<bool> {
    let setup = reader.tracks()[index].decoder_setup(description)?;
    if setup.codec != *b"mp4a" {
        return Ok(false);
    }
    let asc = mp4_probe_config(&setup.configuration)?;
    let Ok(mut probe) = InBandPsProbe::new(asc, setup.sample_rate) else {
        return Ok(false);
    };
    let active_samples = mp4_active_samples(
        &reader.tracks()[index],
        reader.movie_timescale(),
        interval,
        max_packets,
    );
    let mut packet = Vec::new();
    for sample in 0..active_samples {
        check()?;
        if reader.tracks()[index].description_for_sample(sample)
            != reader.tracks()[index]
                .sample_descriptions
                .get((description - 1) as usize)
        {
            probe.reset();
            continue;
        }
        reader.read_packet(index, sample, &mut packet)?;
        if probe.read(&packet).is_err() {
            probe.reset();
            continue;
        }
        if probe.ps_detected() {
            return Ok(true);
        }
    }
    Ok(false)
}

struct Mp4NormalizationRun<D: Mp4SampleDecoder> {
    filter: D::Resampler,
    matrix: Vec<Vec<f64>>,
    next_output: i128,
    input_channels: usize,
    output_channels: usize,
    chunk_frames: usize,
}

impl<D: Mp4SampleDecoder> Mp4NormalizationRun<D> {
    fn new(
        origin: Mp4ExportTime,
        input_rate: u32,
        output_rate: u32,
        input_channels: u16,
        output_channels: u16,
        matrix: Vec<Vec<f64>>,
    ) -> Result<Self> {
        let (whole, phase) = origin.output_origin(output_rate)?;
        let filter = D::new_resampler(input_rate, output_rate, output_channels, phase)?;
        let next_output = whole
            .checked_add(i128::from(filter.output_position()))
            .ok_or_else(|| invalid("MP4 normalization output clock overflow"))?;
        let chunk_frames =
            (4096u64 * u64::from(input_rate) / u64::from(output_rate)).clamp(1, 256) as usize;
        Ok(Self {
            filter,
            matrix,
            next_output,
            input_channels: usize::from(input_channels),
            output_channels: usize::from(output_channels),
            chunk_frames,
        })
    }

    fn publish(
        &mut self,
        output: &mut impl Write,
        window: std::ops::Range<i128>,
        cursor: &mut i128,
    ) -> Result<()> {
        let bytes = self.filter.take_output();
        let stride = self.output_channels * D::SAMPLE_BYTES;
        if !bytes.len().is_multiple_of(stride) {
            return Err(invalid("invalid normalized MP4 PCM stride"));
        }
        let end = self
            .next_output
            .checked_add((bytes.len() / stride) as i128)
            .ok_or_else(|| invalid("MP4 normalization output clock overflow"))?;
        let first = self.next_output.max(window.start).max(*cursor);
        let last = end.min(window.end);
        if first < last {
            if first != *cursor {
                return Err(invalid("MP4 audio edit extends outside available samples"));
            }
            let a = usize::try_from(first - self.next_output)
                .map_err(|_| invalid("MP4 normalization offset overflow"))?;
            let b = usize::try_from(last - self.next_output)
                .map_err(|_| invalid("MP4 normalization offset overflow"))?;
            output.write_all(&bytes[a * stride..b * stride])?;
            *cursor = last;
        }
        self.next_output = end;
        Ok(())
    }

    fn feed<C: AudioDecodeControl>(
        &mut self,
        samples: &[D::Sample],
        output: &mut impl Write,
        window: std::ops::Range<i128>,
        cursor: &mut i128,
        control: &mut C,
    ) -> Result<()> {
        if !samples.len().is_multiple_of(self.input_channels) {
            return Err(invalid("invalid MP4 normalization input stride"));
        }
        let mut mixed = Vec::new();
        mixed
            .try_reserve_exact(self.chunk_frames * self.output_channels * D::SAMPLE_BYTES)
            .map_err(|_| invalid("MP4 normalization allocation failed"))?;
        for chunk in samples.chunks(self.chunk_frames * self.input_channels) {
            control.check()?;
            mixed.clear();
            for frame in chunk.chunks_exact(self.input_channels) {
                for channel in 0..self.output_channels {
                    let value = frame
                        .iter()
                        .enumerate()
                        .map(|(i, &v)| D::sample_to_f64(v) * self.matrix[i][channel])
                        .sum::<f64>();
                    D::push_f64_as_le(value, &mut mixed);
                }
            }
            self.filter.write_all(&mixed)?;
            self.publish(output, window.clone(), cursor)?;
        }
        Ok(())
    }

    fn finish(
        &mut self,
        output: &mut impl Write,
        window: std::ops::Range<i128>,
        cursor: &mut i128,
    ) -> Result<()> {
        self.filter.finish_resampling()?;
        self.publish(output, window, cursor)
    }
}

fn mp4_normalized_silence<C: AudioDecodeControl, D: Mp4SampleDecoder>(
    run: &mut Option<Mp4NormalizationRun<D>>,
    output: &mut impl Write,
    window: std::ops::Range<i128>,
    gap: std::ops::Range<i128>,
    cursor: &mut i128,
    channels: u16,
    control: &C,
) -> Result<()> {
    if let Some(filter) = run {
        filter.finish(output, window.clone(), cursor)?;
    }
    *run = None;
    let first = gap.start.max(window.start);
    let last = gap.end.min(window.end);
    if first < last {
        if *cursor < first {
            return Err(invalid("MP4 audio gap has an uncovered source window"));
        }
        if *cursor < last {
            mp4_write_tail_silence(
                output,
                u64::try_from(last - *cursor).map_err(|_| invalid("MP4 gap silence overflow"))?,
                channels,
                D::SAMPLE_BYTES,
                control,
            )?;
            *cursor = last;
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn decode_mp4_audio_normalized<
    R: Read + Seek,
    C: AudioDecodeControl,
    D: Mp4SampleDecoder,
>(
    mut reader: Mp4TimelineReader<R>,
    output: &mut impl Write,
    interval: Option<(Duration, Duration)>,
    index: usize,
    track: crate::owned_mp4::Track,
    initial_description: u32,
    ps: Vec<bool>,
    decoder: D,
    target: Option<(u32, u16, u32)>,
    control: &mut C,
) -> Result<AudioDecodeStats> {
    let (rate, channels, destination) = target.unwrap_or_else(|| {
        (
            decoder.sample_rate(),
            decoder.channels(),
            decoder.normalization_mask(),
        )
    });
    if rate == 0 || channels == 0 || channels > 8 {
        return Err(invalid(
            "unsupported MP4 normalization destination geometry",
        ));
    }
    let active_samples = mp4_active_samples(
        &track,
        reader.movie_timescale(),
        interval,
        control.packet_limit(),
    );
    let mut opaque_identity = false;
    if destination == 0 && target.is_none() {
        let first = track.decoder_setup(initial_description)?;
        if first.codec == *b"mp4a" {
            let positions = mp4_opaque_positions(&first.configuration).map_err(|e| invalid(&e))?;
            opaque_identity = positions
                .as_ref()
                .is_some_and(|p| p.len() == usize::from(channels));
            for i in 0..track.sample_descriptions.len() {
                if !track.description_used_before((i + 1) as u32, active_samples) {
                    continue;
                }
                let setup = track.decoder_setup((i + 1) as u32)?;
                if setup.codec != *b"mp4a"
                    || setup.channels != channels
                    || mp4_opaque_positions(&setup.configuration).map_err(|e| invalid(&e))?
                        != positions
                {
                    opaque_identity = false;
                    break;
                }
            }
        }
    }
    if destination == 0 && !opaque_identity {
        return Err(invalid(
            "MP4 normalization requires an explicit destination speaker layout",
        ));
    }
    let make_decoder = |id: u32| -> Result<D> {
        let setup = track.decoder_setup(id)?;
        if ps[(id - 1) as usize] {
            D::with_in_band_ps(&setup)
        } else {
            D::new(&setup)
        }
    };
    let matrix_for = |id: u32, d: &D| -> Result<Vec<Vec<f64>>> {
        let setup = track.decoder_setup(id)?;
        if setup.sample_rate != d.sample_rate()
            || (!ps[(id - 1) as usize] && setup.channels != d.channels())
        {
            return Err(invalid(
                "MP4 container and normalization decoder geometry disagree",
            ));
        }
        if opaque_identity {
            return Ok((0..usize::from(channels))
                .map(|i| {
                    (0..usize::from(channels))
                        .map(|j| if i == j { 1. } else { 0. })
                        .collect()
                })
                .collect());
        }
        mp4_normalization_coefficients(
            setup.codec,
            &setup.configuration,
            d.sample_rate(),
            d.channels(),
            d.normalization_mask(),
            ps[(id - 1) as usize],
            channels,
            destination,
        )
        .map_err(|e| invalid(&e))
    };
    for i in 0..track.sample_descriptions.len() {
        control.check()?;
        if track.description_used_before((i + 1) as u32, active_samples) {
            matrix_for((i + 1) as u32, &make_decoder((i + 1) as u32)?)?;
        }
    }
    let packet_clocks =
        mp4_packet_clock_runs(&mut reader, index, control.packet_limit(), || control.check())?;
    let edit_clock = track.resolved_edit_clock(reader.movie_timescale())?;
    let mut timeline =
        Mp4ExportTimeline::new(edit_clock.edits(), track.timescale, edit_clock.timescale)?;
    if track.edits.is_empty() {
        let endpoint = track.presentation_endpoint()?;
        let mut end = Mp4ExportTime::new(endpoint, u128::from(track.timescale))?;
        if track.samples.get(0).is_some_and(|s| s.pts == 0)
            && let Some(last) = packet_clocks.last()
            && last.last == track.samples.len()
            && last.quantized
            && mp4_tick_matches(last.end, endpoint, track.timescale)?
        {
            end = last.end;
        }
        timeline.segments.push(Mp4ExportSegment {
            start: Mp4ExportTime::ZERO,
            end,
            media: Some(Mp4ExportTime::ZERO),
        });
        timeline.end = end;
    }
    let declared_tail = track
        .declared_empty_tail()?
        .map(|tail| -> Result<_> {
            Ok((
                Mp4ExportTime::new(tail.start, u128::from(track.timescale))?,
                Mp4ExportTime::new(tail.end, u128::from(track.timescale))?,
            ))
        })
        .transpose()?;
    let total = timeline.end.ceil_frames(rate)?;
    let boundary = |d: Duration| -> Result<i128> {
        Mp4ExportTime::new(
            i128::try_from(d.as_nanos()).map_err(|_| invalid("MP4 interval overflow"))?,
            1_000_000_000,
        )?
        .ceil_frames(rate)
    };
    let (from, to) = if let Some((a, b)) = interval {
        (boundary(a)?, boundary(b)?.min(total))
    } else {
        (0, total)
    };
    if from >= to {
        return Err(invalid("audio interval contains no samples"));
    }
    let mut stats = AudioDecodeStats {
        sample_frames: 0,
        decoded_frames: 0,
        sample_rate: rate,
        channels,
    };
    let mut opening_decoder = Some(decoder);
    let mut packet = Vec::new();
    for segment in timeline.segments {
        control.check()?;
        if control.packet_limit_reached() {
            break;
        }
        let begin = from.max(segment.start.ceil_frames(rate)?);
        let end = to.min(segment.end.ceil_frames(rate)?);
        if begin >= end {
            continue;
        }
        let Some(media) = segment.media else {
            let stride = usize::from(channels) * D::SAMPLE_BYTES;
            let zeros = [0u8; 4096];
            let mut remaining = u64::try_from(end - begin)
                .ok()
                .and_then(|n| n.checked_mul(stride as u64))
                .ok_or_else(|| invalid("MP4 silence overflow"))?;
            while remaining > 0 {
                control.check()?;
                let n = remaining.min(zeros.len() as u64) as usize;
                output.write_all(&zeros[..n])?;
                remaining -= n as u64;
            }
            stats.sample_frames +=
                u64::try_from(end - begin).map_err(|_| invalid("MP4 output count overflow"))?;
            continue;
        };
        let window = begin..end;
        let mut cursor = begin;
        let mut decoder = if let Some(d) = opening_decoder.take() {
            d
        } else {
            make_decoder(initial_description)?
        };
        let mut active = initial_description;
        let mut native_cursor = 0u64;
        let mut pending = std::collections::VecDeque::new();
        let mut sample_index = 0usize;
        let mut clock_index = 0usize;
        let mut clock_native_cursor = 0u64;
        let mut source_output_index = 0usize;
        let mut last_source_end: Option<Mp4ExportTime> = None;
        let mut terminal_gap: Option<Mp4ExportTime> = None;
        let mut expected: Option<Mp4ExportTime> = None;
        let mut run: Option<Mp4NormalizationRun<D>> = None;
        loop {
            control.check()?;
            if cursor == end {
                break;
            }
            let stop_in_gap = if sample_index > 0
                && track.declared_gap_after(sample_index - 1).is_some()
            {
                let next = track
                    .samples
                    .get(sample_index)
                    .ok_or_else(|| invalid("missing MP4 gap endpoint"))?;
                let stop = Mp4ExportTime::signed_ticks(next.pts, track.timescale)?;
                if segment.start.add(stop.sub(media)?)?.ceil_frames(rate)? >= end {
                    terminal_gap = Some(stop);
                    true
                } else {
                    false
                }
            } else {
                false
            };
            let terminal =
                sample_index == track.samples.len() || control.packet_limit_reached() || stop_in_gap;
            let changing = !terminal && {
                let id = track
                    .samples
                    .get(sample_index)
                    .ok_or_else(|| invalid("missing MP4 sample"))?
                    .description_index;
                track.sample_descriptions[(id - 1) as usize]
                    != track.sample_descriptions[(active - 1) as usize]
            };
            let (start, duration, samples) = if terminal || (changing && !pending.is_empty()) {
                let Some(samples) = decoder.finish_delayed()? else {
                    if !pending.is_empty() {
                        return Err(invalid("delayed MP4 audio left an undrained source window"));
                    }
                    if let Some(mut r) = run.take() {
                        r.finish(output, window.clone(), &mut cursor)?;
                    }
                    break;
                };
                let (start, duration) = pending
                    .pop_front()
                    .ok_or_else(|| invalid("delayed MP4 audio has no source window"))?;
                (start, duration, samples)
            } else {
                let sample = track
                    .samples
                    .get(sample_index)
                    .ok_or_else(|| invalid("missing MP4 sample"))?;
                if changing {
                    if let Some(r) = &mut run {
                        r.finish(output, window.clone(), &mut cursor)?;
                    }
                    if cursor == end {
                        break;
                    }
                    run = None;
                    decoder = make_decoder(sample.description_index)?;
                    active = sample.description_index;
                    native_cursor = 0;
                }
                let declared_start = Mp4ExportTime::signed_ticks(sample.pts, track.timescale)?;
                let declared_duration =
                    Mp4ExportTime::ticks(u64::from(sample.duration), track.timescale)?;
                let gap = sample_index > 0 && track.declared_gap_after(sample_index - 1).is_some();
                if expected.is_some_and(|at| {
                    at != declared_start
                        && !(gap && at.cmp(declared_start) == std::cmp::Ordering::Less)
                }) {
                    return Err(invalid("non-contiguous MP4 audio timeline"));
                }
                expected = Some(declared_start.add(declared_duration)?);
                while packet_clocks
                    .get(clock_index)
                    .is_some_and(|r| sample_index >= r.last)
                {
                    clock_index += 1;
                    clock_native_cursor = 0;
                }
                let clock = packet_clocks
                    .get(clock_index)
                    .filter(|r| sample_index >= r.first)
                    .ok_or_else(|| invalid("missing MP4 packet clock run"))?;
                if gap && pending.is_empty() {
                    let start = last_source_end
                        .ok_or_else(|| invalid("MP4 gap has no preceding PCM window"))?;
                    let stop = if clock.quantized {
                        clock.origin
                    } else {
                        declared_start
                    };
                    let a = segment.start.add(start.sub(media)?)?.ceil_frames(rate)?;
                    let b = segment.start.add(stop.sub(media)?)?.ceil_frames(rate)?;
                    mp4_normalized_silence(
                        &mut run,
                        output,
                        window.clone(),
                        a..b,
                        &mut cursor,
                        channels,
                        control,
                    )?;
                    if cursor == end {
                        break;
                    }
                }
                reader.read_packet(index, sample_index, &mut packet)?;
                let (start, duration, frames) = if clock.quantized {
                    let setup = track
                        .description_for_sample(sample_index)
                        .ok_or_else(|| invalid("missing MP4 clock description"))?;
                    let frames = mp4_coded_packet_frames(
                        &setup.codec,
                        &setup.configuration,
                        decoder.sample_rate(),
                        setup.channels,
                        setup.bit_depth,
                        packet.len(),
                        Some(&packet),
                    )
                    .map_err(|e| invalid(&e))?;
                    (
                        clock.origin.add(Mp4ExportTime::new(
                            i128::from(clock_native_cursor),
                            u128::from(decoder.sample_rate()),
                        )?)?,
                        Mp4ExportTime::new(i128::from(frames), u128::from(decoder.sample_rate()))?,
                        frames,
                    )
                } else {
                    let frames = declared_duration.ceil_frames(decoder.sample_rate())?;
                    if frames <= 0
                        || Mp4ExportTime::new(frames, u128::from(decoder.sample_rate()))?
                            != declared_duration
                    {
                        return Err(invalid("MP4 audio packet clock is not sample aligned"));
                    }
                    (
                        declared_start,
                        declared_duration,
                        u64::try_from(frames)
                            .map_err(|_| invalid("MP4 native frame count overflow"))?,
                    )
                };
                let samples = decoder.decode_timed(&packet, native_cursor, frames)?;
                native_cursor = native_cursor
                    .checked_add(frames)
                    .ok_or_else(|| invalid("MP4 native clock overflow"))?;
                clock_native_cursor = clock_native_cursor
                    .checked_add(frames)
                    .ok_or_else(|| invalid("MP4 run native clock overflow"))?;
                control.packet(packet.len())?;
                sample_index += 1;
                if decoder.delayed() {
                    pending.push_back((start, duration));
                    if pending.len() > 3 {
                        return Err(invalid("delayed MP4 source lookahead exceeded"));
                    }
                    let Some(samples) = samples else {
                        continue;
                    };
                    let (start, duration) = pending
                        .pop_front()
                        .ok_or_else(|| invalid("delayed MP4 source identity mismatch"))?;
                    (start, duration, samples)
                } else {
                    (
                        start,
                        duration,
                        samples.ok_or_else(|| invalid("immediate MP4 audio returned no PCM"))?,
                    )
                }
            };
            let input_channels = usize::from(decoder.channels());
            let frames = usize::try_from(duration.ceil_frames(decoder.sample_rate())?)
                .map_err(|_| invalid("MP4 native duration overflow"))?;
            if !samples.len().is_multiple_of(input_channels)
                || frames == 0
                || frames > samples.len() / input_channels
            {
                return Err(invalid(
                    "MP4 audio packet duration disagrees with decoded samples",
                ));
            }
            if source_output_index > 0
                && track.declared_gap_after(source_output_index - 1).is_some()
            {
                let previous =
                    last_source_end.ok_or_else(|| invalid("MP4 gap has no decoded predecessor"))?;
                let a = segment.start.add(previous.sub(media)?)?.ceil_frames(rate)?;
                let b = segment.start.add(start.sub(media)?)?.ceil_frames(rate)?;
                mp4_normalized_silence(
                    &mut run,
                    output,
                    window.clone(),
                    a..b,
                    &mut cursor,
                    channels,
                    control,
                )?;
            }
            last_source_end = Some(start.add(duration)?);
            source_output_index += 1;
            if run.is_none() {
                let origin = segment.start.add(start.sub(media)?)?;
                run = Some(Mp4NormalizationRun::new(
                    origin,
                    decoder.sample_rate(),
                    rate,
                    decoder.channels(),
                    channels,
                    matrix_for(active, &decoder)?,
                )?);
            }
            run.as_mut().unwrap().feed(
                &samples[..frames * input_channels],
                output,
                window.clone(),
                &mut cursor,
                control,
            )?;
            stats.decoded_frames += 1;
        }
        if cursor != end
            && !control.packet_limit_reached()
            && let (Some(stop), Some(start)) = (terminal_gap, last_source_end)
        {
            let a = segment.start.add(start.sub(media)?)?.ceil_frames(rate)?;
            let b = segment.start.add(stop.sub(media)?)?.ceil_frames(rate)?;
            mp4_normalized_silence(
                &mut run,
                output,
                window.clone(),
                a..b,
                &mut cursor,
                channels,
                control,
            )?;
        }
        if cursor != end && !control.packet_limit_reached() {
            let tail = declared_tail
                .map(|(start, stop)| -> Result<_> {
                    Ok((
                        segment.start.add(start.sub(media)?)?.ceil_frames(rate)?,
                        segment.start.add(stop.sub(media)?)?.ceil_frames(rate)?,
                    ))
                })
                .transpose()?;
            if sample_index == track.samples.len()
                && tail.is_some_and(|(start, stop)| cursor >= start && end <= stop)
            {
                mp4_write_tail_silence(
                    output,
                    u64::try_from(end - cursor)
                        .map_err(|_| invalid("MP4 tail output count overflow"))?,
                    channels,
                    D::SAMPLE_BYTES,
                    control,
                )?;
                cursor = end;
            } else {
                return Err(invalid("MP4 audio edit extends outside available samples"));
            }
        }
        stats.sample_frames = stats
            .sample_frames
            .checked_add(
                u64::try_from(cursor - begin).map_err(|_| invalid("MP4 output count overflow"))?,
            )
            .ok_or_else(|| invalid("MP4 output count overflow"))?;
    }
    if stats.sample_frames == 0 {
        return Err(invalid("MP4 audio edit contains no samples"));
    }
    Ok(stats)
}

pub fn mp4_normalized_geometry<R: Read + Seek>(
    reader: &mut Mp4TimelineReader<R>,
    index: usize,
    max_packets: Option<u64>,
) -> Result<Option<(u32, u16, u32)>> {
    mp4_normalized_geometry_with_interval(reader, index, None, max_packets)
}

pub fn mp4_normalized_geometry_with_interval<R: Read + Seek>(
    reader: &mut Mp4TimelineReader<R>,
    index: usize,
    interval: Option<(Duration, Duration)>,
    max_packets: Option<u64>,
) -> Result<Option<(u32, u16, u32)>> {
    mp4_normalized_geometry_controlled_with_interval(reader, index, interval, max_packets, || {
        Ok(())
    })
}

pub fn mp4_normalized_geometry_controlled<R: Read + Seek>(
    reader: &mut Mp4TimelineReader<R>,
    index: usize,
    max_packets: Option<u64>,
    check: impl FnMut() -> Result<()>,
) -> Result<Option<(u32, u16, u32)>> {
    mp4_normalized_geometry_controlled_with_interval(reader, index, None, max_packets, check)
}

pub fn mp4_normalized_geometry_controlled_with_interval<R: Read + Seek>(
    reader: &mut Mp4TimelineReader<R>,
    index: usize,
    interval: Option<(Duration, Duration)>,
    max_packets: Option<u64>,
    mut check: impl FnMut() -> Result<()>,
) -> Result<Option<(u32, u16, u32)>> {
    let track = &reader.tracks()[index];
    let first = track
        .samples
        .get(0)
        .ok_or_else(|| invalid("MP4 audio has no samples"))?
        .description_index;
    let setup = track.decoder_setup(first)?;
    let ps = negotiate_mp4_description_ps_with_interval(
        reader,
        index,
        first,
        interval,
        max_packets,
        &mut check,
    )?;
    let decoder = if ps {
        Mp4TimelineDecoder::with_in_band_ps(&setup)?
    } else {
        Mp4TimelineDecoder::new(&setup)?
    };
    let initial = (
        decoder.sample_rate(),
        decoder.channels(),
        decoder.normalization_mask(),
    );
    let active_samples = mp4_active_samples(
        &reader.tracks()[index],
        reader.movie_timescale(),
        interval,
        max_packets,
    );
    for i in 0..reader.tracks()[index].sample_descriptions.len() {
        check()?;
        let track = &reader.tracks()[index];
        if !track.description_used_before((i + 1) as u32, active_samples) {
            continue;
        }
        let setup = track.decoder_setup((i + 1) as u32)?;
        let ps = negotiate_mp4_description_ps_with_interval(
            reader,
            index,
            (i + 1) as u32,
            interval,
            max_packets,
            &mut check,
        )?;
        let next = if ps {
            Mp4TimelineDecoder::with_in_band_ps(&setup)?
        } else {
            Mp4TimelineDecoder::new(&setup)?
        };
        if (
            next.sample_rate(),
            next.channels(),
            next.normalization_mask(),
        ) != initial
        {
            return Ok(Some(initial));
        }
    }
    Ok(None)
}

#[cfg(test)]
mod admission_tests {
    use super::*;
    #[test]
    fn mp4_aac_pcm_admits_index_decoder_and_edit_checkpoint() {
        let source = std::fs::read(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../tests/fixtures/audio/aac-native-edit.m4a"),
        )
        .unwrap();
        let decode = |options: &CopyOptions| {
            let mut pcm = Vec::new();
            let stats =
                decode_mp4_audio_pcm(std::io::Cursor::new(&source), &mut pcm, None, options);
            (stats, pcm)
        };
        let (expected, pcm) = decode(&CopyOptions::default());
        let expected = expected.unwrap();
        let (actual, admitted) = decode(&CopyOptions {
            max_controlled_bytes: Some(32 * 1024 * 1024),
            ..Default::default()
        });
        assert_eq!(actual.unwrap(), expected);
        assert_eq!(admitted, pcm);
        for limit in [
            1,
            crate::owned_aac::stream::decode_admission_bytes(expected.channels).unwrap(),
        ] {
            let (error, output) = decode(&CopyOptions {
                max_controlled_bytes: Some(limit),
                ..Default::default()
            });
            assert!(
                error
                    .unwrap_err()
                    .to_string()
                    .contains("controlled memory budget exceeded")
            );
            assert!(output.is_empty());
        }
    }
}

/// Decode QuickTime PCM without narrowing integer32 or IEEE float64 samples.
/// Encoded timelines and controls match the normalized f32 API.
pub fn decode_mp4_pcm_f64<R: Read + Seek>(
    source: R,
    output: &mut impl Write,
    interval: Option<(Duration, Duration)>,
    options: &CopyOptions,
) -> Result<AudioDecodeStats> {
    precise::decode(source, output, interval, options, None)
}

pub(crate) fn decode_mp4_pcm_f64_targeted<R: Read + Seek>(
    source: R,
    output: &mut impl Write,
    interval: Option<(Duration, Duration)>,
    options: &CopyOptions,
    target: Option<(u32, u16, u32)>,
) -> Result<AudioDecodeStats> {
    precise::decode(source, output, interval, options, target)
}

pub(crate) fn decode_mp4_pcm_f64_reader_targeted<R: Read + Seek>(
    reader: Mp4TimelineReader<R>,
    output: &mut impl Write,
    interval: Option<(Duration, Duration)>,
    options: &CopyOptions,
    target: Option<(u32, u16, u32)>,
) -> Result<AudioDecodeStats> {
    precise::decode_reader(reader, output, interval, options, target)
}

mod precise {
    use super::*;

    struct PreciseMp4PcmDecoder(crate::owned_pcm_decoder::PcmDecoder);

    impl Mp4SampleDecoder for PreciseMp4PcmDecoder {
        type Sample = f64;
        type Resampler = crate::owned_resample_f64::Resampler<Vec<u8>>;
        const SAMPLE_BYTES: usize = 8;

        fn sample_to_f64(sample: f64) -> f64 {
            sample
        }
        fn push_f64_as_le(value: f64, out: &mut Vec<u8>) {
            out.extend_from_slice(&value.to_le_bytes());
        }
        fn push_sample_le(sample: f64, out: &mut Vec<u8>) {
            out.extend_from_slice(&sample.to_le_bytes());
        }
        fn new_resampler(
            input_rate: u32,
            output_rate: u32,
            channels: u16,
            phase: (u128, u128),
        ) -> std::io::Result<Self::Resampler> {
            crate::owned_resample_f64::Resampler::new_with_origin(
                Vec::new(),
                input_rate,
                output_rate,
                channels,
                0,
                phase,
            )
        }
        fn with_in_band_ps(_: &crate::owned_mp4::Track) -> Result<Self> {
            Err(invalid("PCM reader cannot select AAC PS"))
        }
        fn new(track: &crate::owned_mp4::Track) -> Result<Self> {
            Ok(Self(
                crate::owned_pcm_decoder::PcmDecoder::from_mp4(track)
                    .map_err(|e| invalid(&e.to_string()))?,
            ))
        }
        fn sample_rate(&self) -> u32 {
            self.0.sample_rate()
        }
        fn channels(&self) -> u16 {
            self.0.channels()
        }
        fn normalization_mask(&self) -> u32 {
            crate::owned_pcm_channels::standard_mask(self.channels()).unwrap_or(0) as u32
        }
        fn checkpoint(&self) -> Option<Mp4AacCheckpoint> {
            None
        }
        fn restore_checkpoint(&mut self, _: &Mp4AacCheckpoint) -> Result<bool> {
            Ok(false)
        }
        fn delayed(&self) -> bool {
            false
        }
        fn finish_delayed(&mut self) -> Result<Option<Vec<f64>>> {
            Ok(None)
        }
        fn decode_timed(
            &mut self,
            bytes: &[u8],
            _pts: u64,
            _duration: u64,
        ) -> Result<Option<Vec<f64>>> {
            self.0
                .decode_pcm_f64(bytes)
                .map(Some)
                .map_err(|e| invalid(&e.to_string()))
        }
    }

    pub(super) fn decode<R: Read + Seek>(
        source: R,
        output: &mut impl Write,
        interval: Option<(Duration, Duration)>,
        options: &CopyOptions,
        target: Option<(u32, u16, u32)>,
    ) -> Result<AudioDecodeStats> {
        let mut control = CopyProgress::new(options);
        control.check()?;
        if !options.metadata_set.is_empty()
            || !options.metadata_delete.is_empty()
            || !options.stream_metadata_set.is_empty()
            || !options.stream_metadata_delete.is_empty()
        {
            return Err(invalid("raw PCM stream cannot apply metadata mutations"));
        }
        let selected = match options.streams.as_slice() {
            [] => None,
            [index] => Some(*index),
            _ => return Err(invalid("select exactly one audio stream")),
        };
        if let Some(hook) = &options.progress {
            hook.emit(control.event);
        }
        control.check()?;
        let metadata_bytes = crate::owned_mp4::Limits::default()
            .metadata_bytes
            .min(options.max_controlled_bytes.unwrap_or(usize::MAX));
        let mut reader = Mp4TimelineReader::open_indexed(
            source,
            crate::owned_mp4::Limits {
                metadata_bytes,
                ..Default::default()
            },
        )
        .map_err(|error| {
            if metadata_bytes < crate::owned_mp4::Limits::default().metadata_bytes
                && error.to_string().contains("MP4 metadata exceeds budget")
            {
                invalid("controlled memory budget exceeded: MP4 metadata exceeds budget")
            } else {
                error.into()
            }
        })?;
        reader.restrict_packet_bytes(options.max_packet_bytes);
        let index = mp4_audio_index(&reader, selected)?;
        admit_audio_reader_with_interval(&reader, index, interval, options, target)?;
        decode_mp4_audio_reader_generic::<R, CopyProgress<'_>, PreciseMp4PcmDecoder>(
            reader,
            output,
            interval,
            selected,
            &mut control,
            target,
        )
    }

    pub(super) fn decode_reader<R: Read + Seek>(
        reader: Mp4TimelineReader<R>,
        output: &mut impl Write,
        interval: Option<(Duration, Duration)>,
        options: &CopyOptions,
        target: Option<(u32, u16, u32)>,
    ) -> Result<AudioDecodeStats> {
        let mut control = CopyProgress::new(options);
        control.check()?;
        if !options.metadata_set.is_empty()
            || !options.metadata_delete.is_empty()
            || !options.stream_metadata_set.is_empty()
            || !options.stream_metadata_delete.is_empty()
        {
            return Err(invalid("raw PCM stream cannot apply metadata mutations"));
        }
        let selected = match options.streams.as_slice() {
            [] => None,
            [index] => Some(*index),
            _ => return Err(invalid("select exactly one audio stream")),
        };
        if let Some(hook) = &options.progress {
            hook.emit(control.event);
        }
        control.check()?;
        decode_mp4_audio_reader_generic::<R, CopyProgress<'_>, PreciseMp4PcmDecoder>(
            reader,
            output,
            interval,
            selected,
            &mut control,
            target,
        )
    }
}

#[cfg(test)]
#[path = "owned_mp4_audio_gap_tests.rs"]
mod gap_tests;
