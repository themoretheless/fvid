//! Owned MP4 AAC/ALAC/PCM/ADPCM/MACE presentation decoding, including silence and repeated edits.
use crate::owned_aac::aac_ps_native::InBandPsProbe;
use crate::owned_codec_config::aac_specific_config as mp4_probe_config;
pub(crate) enum Mp4AacCheckpoint {
    Aac(crate::owned_aac::AacCheckpoint),
    Ps(crate::owned_aac::aac_ps_native::Checkpoint),
}
pub use crate::owned_matroska_audio::{AudioDecodeStats, Error};
use crate::owned_matroska_audio::{DecodeProgress, invalid};
use crate::owned_mp4::Mp4Reader as Mp4TimelineReader;
use crate::owned_mp4_audio_schedule::AudioTimeline as Mp4AudioSchedule;
use fvid_control::CopyOptions;
use std::{
    io::{Read, Seek, Write},
    time::Duration,
};
type Result<T> = std::result::Result<T, Error>;
impl From<crate::owned_mp4::Error> for Error {
    fn from(e: crate::owned_mp4::Error) -> Self {
        invalid(&e.to_string())
    }
}
pub(crate) enum Mp4TimelineDecoder {
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
    const SAMPLE_BYTES: usize = 4;
    pub(crate) fn with_in_band_ps(track: &crate::owned_mp4::Track) -> Result<Self> {
        Ok(Self::Ps(Box::new(crate::owned_aac::aac_ps_native::NativePsAacDecoder::new_with_in_band_ps(
            mp4_probe_config(&track.configuration)?, track.sample_rate,
        )?)))
    }

    pub(crate) fn checkpoint(&self) -> Option<Mp4AacCheckpoint> {
        match self {
            Self::Aac(d) => Some(Mp4AacCheckpoint::Aac(d.checkpoint())),
            Self::Ps(d) => Some(Mp4AacCheckpoint::Ps(d.checkpoint())),
            _ => None,
        }
    }
    pub(crate) fn restore_checkpoint(&mut self, state: &Mp4AacCheckpoint) -> Result<bool> {
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

    pub(crate) fn new(track: &crate::owned_mp4::Track) -> Result<Self> {
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
            b"alac" => {
                Ok(Self::Alac(crate::owned_alac::AlacDecoder::new(
                    &track.configuration,
                    track.sample_rate,
                    track.channels,
                )?))
            }
            b"raw " | b"sowt" | b"twos" | b"in24" | b"in32" | b"fl32" | b"fl64" => Ok(Self::Pcm(
                crate::owned_pcm_decoder::PcmDecoder::from_mp4(track)
                    .map_err(|e| invalid(&e.to_string()))?,
            )),
            b"MAC3" | b"MAC6" => Ok(Self::Mace(crate::owned_mace::MaceDecoder::new(
                if track.codec==*b"MAC3" {crate::owned_mace::MaceCoding::ThreeToOne}else{crate::owned_mace::MaceCoding::SixToOne},
                track.sample_rate,track.channels,
            ).map_err(|e|invalid(&e.to_string()))?)),
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
    pub(crate) fn sample_rate(&self) -> u32 {
        match self {
            Self::Aac(d) => d.sample_rate(),
            Self::Ps(d) => d.sample_rate(),
            Self::Alac(d) => d.sample_rate(),
            Self::Pcm(d) => d.sample_rate(),
            Self::Ima4(d) => d.sample_rate(),
            Self::Mace(d)=>d.sample_rate(),
            Self::ImaWav(d) => d.sample_rate(),
            Self::MsAdpcm(d) => d.sample_rate(),
        }
    }
    pub(crate) fn channels(&self) -> u16 {
        match self {
            Self::Aac(d) => u16::from(d.channels()),
            Self::Ps(d) => u16::from(d.channels()),
            Self::Alac(d) => d.channels(),
            Self::Pcm(d) => d.channels(),
            Self::Ima4(d) => d.channels(),
            Self::Mace(d)=>d.channels(),
            Self::ImaWav(d) => d.channels(),
            Self::MsAdpcm(d) => d.channels(),
        }
    }
    pub(crate) fn channel_mask(&self) -> u32 {
        match self {
            Self::Aac(d) => d.channel_mask(),
            Self::Ps(d) => d.channel_mask(),
            Self::Mace(d)=>crate::owned_pcm_channels::standard_mask(d.channels()).unwrap_or(0)as u32,
            Self::ImaWav(d) => {
                crate::owned_pcm_channels::standard_mask(d.channels()).unwrap_or(0) as u32
            }
            Self::MsAdpcm(d) => {
                crate::owned_pcm_channels::standard_mask(d.channels()).unwrap_or(0) as u32
            }
            Self::Ima4(d) => {
                crate::owned_pcm_channels::standard_mask(d.channels()).unwrap_or(0) as u32
            }
            Self::Pcm(d) => {
                crate::owned_pcm_channels::standard_mask(d.channels()).unwrap_or(0) as u32
            }
            Self::Alac(d) => d.channel_mask(),
        }
    }
    fn normalization_mask(&self) -> u32 { self.channel_mask() }
    fn delayed(&self) -> bool {
        matches!(self, Self::Ps(_)) || matches!(self,Self::Aac(d) if d.delayed())
    }
    fn finish_delayed(&mut self) -> Result<Option<Vec<f32>>> {
        match self {
            Self::Ps(d) => Ok(d.finish()?.map(|f| f.pcm)),
            Self::Aac(d) => Ok(d.finish()?.map(|f|f.samples)),
            _ => Ok(None),
        }
    }
    fn decode_timed(&mut self, bytes: &[u8], pts:u64,duration:u64) -> Result<Option<Vec<f32>>> {
        let pcm = match self {
            Self::Ps(d) => return Ok(d.decode(bytes)?.map(|f| f.pcm)),
            Self::Aac(d) => return Ok(d.decode_timed(bytes,i64::try_from(pts).map_err(|_|invalid("AAC timestamp overflow"))?,duration)?.map(|f|f.samples)),
            Self::Alac(d) => d.decode_pcm(bytes)?,
            Self::Pcm(d) => d.decode_pcm(bytes).map_err(|e| invalid(&e.to_string()))?,
            Self::Ima4(d) => d.decode_pcm(bytes).map_err(|e| invalid(&e.to_string()))?,
            Self::Mace(d)=>d.decode_pcm(bytes).map_err(|e|invalid(&e.to_string()))?,
            Self::ImaWav(d) => d.decode_pcm(bytes).map_err(|e| invalid(&e.to_string()))?,
            Self::MsAdpcm(d) => d.decode_pcm(bytes).map_err(|e| invalid(&e.to_string()))?,
        };
        Ok(Some(pcm))
    }
}
pub(crate) fn mp4_audio_index<R: Read + Seek>(
    reader: &Mp4TimelineReader<R>,
    selected: Option<usize>,
) -> Result<usize> {
    let indices: Vec<_> = if selected.is_none() {
        reader.automatic_track_indices(&[*b"soun"])
    } else {
        reader.tracks().iter().enumerate().filter_map(|(index,t)|(t.handler==*b"soun").then_some(index)).collect()
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
        b"raw " | b"sowt" | b"twos" | b"in24" | b"in32" | b"fl32" | b"fl64" => {
            crate::owned_pcm_decoder::PcmDecoder::from_mp4(track)
                .map_err(|e| invalid(&e.to_string()))?;
            16 * 1024
        }
        b"MAC3" | b"MAC6" => {
            crate::owned_mace::MaceDecoder::new(
                if track.codec==*b"MAC3" {crate::owned_mace::MaceCoding::ThreeToOne}else{crate::owned_mace::MaceCoding::SixToOne},
                track.sample_rate,track.channels,
            ).map_err(|e|invalid(&e.to_string()))?;
            16*1024
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
pub(crate) fn admit_audio_reader<R: Read + Seek>(
    reader: &Mp4TimelineReader<R>,
    index: usize,
    options: &CopyOptions,
) -> Result<()> {
    admit_audio_reader_targeted(reader,index,options,None)
}
pub(crate) fn admit_audio_reader_targeted<R: Read + Seek>(
    reader: &Mp4TimelineReader<R>,
    index: usize,
    options: &CopyOptions,
    target: Option<(u32,u16,u32)>,
) -> Result<()> {
    let Some(limit) = options.max_controlled_bytes else {
        return Ok(());
    };
    let track = reader
        .tracks()
        .get(index)
        .ok_or_else(|| invalid("selected audio stream is absent"))?;
    let mut decoder = 0usize;
    let mut checkpoint = 0usize;
    let mut configuration = 0usize;
    for i in 0..track.sample_descriptions.len() {
        if !(0..track.samples.len()).any(|n| track.samples.get(n).is_some_and(|s| s.description_index as usize == i+1)) { continue; }
        let setup = track.decoder_setup((i+1) as u32)?;
        let state = mp4_decoder_admission(&setup)?;
        decoder = decoder.max(state);
        if setup.codec == *b"mp4a" { checkpoint = checkpoint.max(state); }
        configuration = configuration.max(setup.configuration.len());
    }
    // Active plus replacement/restore state and one retained AAC checkpoint.
    // For constant AAC this remains the previous three-state admission bound.
    let decoder = decoder.checked_mul(2).and_then(|n| n.checked_add(checkpoint))
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
    if target.is_some() || (0..track.samples.len()).any(|n|track.description_for_sample(n)!=track.description_for_sample(0)) {
        let first=track.description_for_sample(0).ok_or_else(||invalid("MP4 audio has no samples"))?;
        let output_rate=target.map_or(first.sample_rate,|t|t.0);
        let output_channels=usize::from(target.map_or(first.channels.max(2),|t|t.1));
        let mut normalization=0usize;
        for description in &track.sample_descriptions {
            if description.sample_rate==0 || output_rate==0 {return Err(invalid("zero MP4 normalization clock"));}
            let radius=usize::try_from(crate::owned_resample::input_radius(description.sample_rate,output_rate)).map_err(|_|overflow())?;
            // Geometric queue capacity, f64 slots (also bounds f32), four scalar
            // work arrays, mixed input, and a drained output block coexist.
            let ring=radius.checked_mul(4).and_then(|n|n.checked_add(16)).and_then(|n|n.checked_mul(255*8)).ok_or_else(overflow)?;
            let frames=usize::try_from(u64::from(output_rate).div_ceil(u64::from(description.sample_rate))).map_err(|_|overflow())?.max(4096);
            let buffers=frames.checked_mul(output_channels).and_then(|n|n.checked_mul(8*4)).and_then(|n|n.checked_add(4*255*8+256*8*output_channels)).ok_or_else(overflow)?;
            normalization=normalization.max(ring.checked_add(buffers).ok_or_else(overflow)?);
        }
        add(normalization)?;
    }

    for bytes in [
        track.name.len(),
        track.language.len(),
        track.configuration.len(),
    ] {
        add(bytes)?;
    }
    add(track.sample_descriptions.len().checked_mul(std::mem::size_of::<crate::owned_mp4::SampleDescription>()).ok_or_else(overflow)?)?;
    for description in &track.sample_descriptions {add(description.configuration.len())?;}
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
        crate::owned_mp4::SampleIndex::Expanded(samples) => (
            samples.len(),
            std::mem::size_of::<crate::owned_mp4::Sample>(),
        ),
        crate::owned_mp4::SampleIndex::Uniform(index) => (
            index.runs.len(),
            std::mem::size_of::<crate::owned_mp4::FrameRun>(),
        ),
    };
    add(records.checked_mul(width).ok_or_else(overflow)?)?;
    let largest_packet = (0..track.samples.len())
        .filter_map(|i| track.samples.get(i))
        .map(|s| s.size as usize)
        .max()
        .unwrap_or(0);
    // The reusable packet Vec may grow geometrically while reading larger packets.
    let expansion = track.sample_descriptions.iter().map(|d| match &d.codec {
        b"MAC3" | b"MAC6" => 64,
        b"raw " | b"sowt" | b"twos" | b"in24" | b"in32" | b"fl32" | b"fl64" | b"ima4" | b"ms\x00\x11" | b"ms\x00\x02" => 10,
        _ => 2,
    }).max().unwrap_or(2);
    add(largest_packet.checked_mul(expansion).ok_or_else(overflow)?)?;
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
    decode_mp4_audio_pcm_targeted(source,output,interval,options,None)
}
pub(crate) fn decode_mp4_audio_pcm_targeted<R: Read + Seek>(
    source: R,
    output: &mut impl Write,
    interval: Option<(Duration, Duration)>,
    options: &CopyOptions,
    target: Option<(u32,u16,u32)>,
) -> Result<AudioDecodeStats> {
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
    let mut control = DecodeProgress {
        options,
        event: fvid_control::ProgressEvent {
            packets: 0,
            payload_bytes: 0,
            done: false,
        },
    };
    control.check()?;
    if let Some(hook) = &options.progress {
        hook.emit(control.event);
    }
    control.check()?;
    let reader = Mp4TimelineReader::open_indexed(
        source,
        crate::owned_mp4::Limits {
            packet_bytes: options.max_packet_bytes,
            ..Default::default()
        },
    )?;
    let index = mp4_audio_index(&reader, selected)?;
    admit_audio_reader_targeted(&reader, index, options,target)?;
    decode_mp4_audio_reader_targeted(reader, output, interval, selected, &mut control,target)
}
use crate::owned_mp4_audio_normalization::coefficients as mp4_normalization_coefficients;
type Mp4NormalizationTrack = crate::owned_mp4::Track;
type Mp4PcmScalar = f32;
type Mp4OutputResampler = crate::owned_resample::Resampler<Vec<u8>>;
#[allow(dead_code)] // Export uses only the arithmetic subset of the shared playback clock.
mod mp4_export_clock {
    use super::{Error, Result, invalid};
    use crate::owned_mp4::Edit;
    use std::{cmp::Ordering, time::Duration};
    include!("owned_mp4_edit_clock_impl.rs");
}
include!("owned_mp4_audio_timeline_impl.rs");

pub(crate) fn mp4_normalized_geometry<R:std::io::Read+std::io::Seek>(reader:&mut Mp4TimelineReader<R>,index:usize)->Result<Option<(u32,u16,u32)>> {
    let track=&reader.tracks()[index];
    let first=track.samples.get(0).ok_or_else(||invalid("MP4 audio has no samples"))?.description_index;
    let setup=track.decoder_setup(first)?;
    let ps=negotiate_mp4_description_ps(reader,index,first,||Ok(()))?;
    let decoder=if ps {Mp4TimelineDecoder::with_in_band_ps(&setup)?}else{Mp4TimelineDecoder::new(&setup)?};
    let initial=(decoder.sample_rate(),decoder.channels(),decoder.normalization_mask());
    for i in 0..reader.tracks()[index].sample_descriptions.len() {
        let track=&reader.tracks()[index];
        if !(0..track.samples.len()).any(|n|track.samples.get(n).is_some_and(|s|s.description_index as usize==i+1)) {continue;}
        let setup=track.decoder_setup((i+1)as u32)?;
        let ps=negotiate_mp4_description_ps(reader,index,(i+1)as u32,||Ok(()))?;
        let next=if ps {Mp4TimelineDecoder::with_in_band_ps(&setup)?}else{Mp4TimelineDecoder::new(&setup)?};
        if (next.sample_rate(),next.channels(),next.normalization_mask())!=initial {return Ok(Some(initial));}
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
    precise::decode(source, output, interval, options,None)
}
pub(crate) fn decode_mp4_pcm_f64_targeted<R:Read+Seek>(source:R,output:&mut impl Write,interval:Option<(Duration,Duration)>,options:&CopyOptions,target:Option<(u32,u16,u32)>)->Result<AudioDecodeStats> {
    precise::decode(source,output,interval,options,target)
}

mod precise {
    use super::*;
    struct Mp4TimelineDecoder(crate::owned_pcm_decoder::PcmDecoder);
    impl Mp4TimelineDecoder {
        const SAMPLE_BYTES: usize = 8;
        fn with_in_band_ps(_: &crate::owned_mp4::Track) -> Result<Self> { Err(invalid("PCM reader cannot select AAC PS")) }

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
        fn channel_mask(&self) -> u32 {
            crate::owned_pcm_channels::standard_mask(self.channels()).unwrap_or(0) as u32
        }
        fn checkpoint(&self) -> Option<Mp4AacCheckpoint> {
            None
        }
        fn restore_checkpoint(&mut self, _: &Mp4AacCheckpoint) -> Result<bool> {
            Ok(false)
        }
        fn normalization_mask(&self) -> u32 { self.channel_mask() }
        fn delayed(&self) -> bool {
            false
        }
        fn finish_delayed(&mut self) -> Result<Option<Vec<f64>>> {
            Ok(None)
        }
        fn decode_timed(&mut self, bytes: &[u8], _pts:u64,_duration:u64) -> Result<Option<Vec<f64>>> {
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
        target: Option<(u32,u16,u32)>,
    ) -> Result<AudioDecodeStats> {
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
        let mut control = DecodeProgress {
            options,
            event: fvid_control::ProgressEvent {
                packets: 0,
                payload_bytes: 0,
                done: false,
            },
        };
        control.check()?;
        if let Some(hook) = &options.progress {
            hook.emit(control.event);
        }
        control.check()?;
        let reader = Mp4TimelineReader::open_indexed(
            source,
            crate::owned_mp4::Limits {
                packet_bytes: options.max_packet_bytes,
                ..Default::default()
            },
        )?;
        let index = mp4_audio_index(&reader, selected)?;
        admit_audio_reader_targeted(&reader, index, options,target)?;
        decode_mp4_audio_reader_targeted(reader, output, interval, selected, &mut control,target)
    }
    type Mp4PcmScalar = f64;
    type Mp4OutputResampler = crate::owned_resample_f64::Resampler<Vec<u8>>;
    include!("owned_mp4_audio_timeline_impl.rs");
}
