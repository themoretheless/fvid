//! Sequential AAC-LC/HE-AAC decoding to caller-owned interleaved float32 PCM.
use super::{
    NativeAacDecoder as AdtsPacketDecoder, Result,
    aac_ps_native::{InBandPsProbe as AdtsPsProbe, NativePsAacDecoder as AdtsPsDecoder},
    adts::StreamReader as AdtsStreamReader,
    config::AudioSpecificConfig as AdtsAudioConfig,
    invalid,
};
use crate::owned_audio_control::{AudioDecodeControl, CopyProgress};
use fvid_control::CopyOptions;
use std::{
    io::{Read, Write},
    time::Duration,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AudioDecodeStats {
    pub sample_frames: u64,
    pub decoded_frames: u64,
    pub sample_rate: u32,
    pub channels: u16,
}

/// Conservative admission estimate for AAC-LC decoder payload and packet scratch.
/// Each channel/coupling reserves 204 KiB at the largest 1024-sample geometry:
/// 126 bytes/sample of immutable tables/windows, 58 of synthesis scratch and
/// overlap, 8 of rollback history, 8 of mono PCM and 4 of interleaved PCM.
/// Another 128 KiB per state covers nested channel/TNS
/// syntax and coupling gain lists. 16 coupling slots and two temporary states
/// cover construction and parsing before duplicate/layout validation. The fixed
/// reserve includes bounded ADTS/ASC/PCE storage, vector headers and file I/O.
/// This is an admission estimate, not a process RSS or allocator-header limit.
/// Keep these bounds in sync when adding larger frame sizes or new AAC tools.
pub(crate) fn decode_admission_bytes(channels: u16) -> Result<usize> {
    let states = usize::from(channels)
        .checked_add(18)
        .ok_or_else(|| invalid("AAC memory estimate overflow"))?;
    let estimated = states
        .checked_mul((204 + 128) * 1024)
        .and_then(|bytes| bytes.checked_add(256 * 1024))
        .ok_or_else(|| invalid("AAC memory estimate overflow"))?;
    Ok(estimated)
}

/// ASC-aware admission for container AAC. In addition to the LC reserve,
/// SBR charges 2 MiB per output channel and configured CCE: 12 complex row buffers at 64x64x16 bytes,
/// eight 64 KiB history/transaction reserves, twelve 5x64x64-byte
/// parameter/level buffers, and four 16 KiB PCM buffers. This rounds their
/// 1.55 MiB sum upward for vector growth/headers and small frequency tables.
/// Bounds cover 960/1024 core frames, at most five envelopes, 64 QMF bands,
/// nested transactional DSP clones and output interleaving; not process RSS.
/// Callers retaining additional whole-decoder checkpoints charge this estimate
/// separately. Revisit the bound when enlarging SBR syntax or DSP geometry.
pub(crate) fn decode_config_admission_bytes(asc: &[u8], output_rate: u32) -> Result<usize> {
    let config = super::config::AudioSpecificConfig::parse(asc)?;
    config.resolve_output_rate(output_rate)?;
    let mut bytes = decode_admission_bytes(u16::from(config.core.channels))?;
    if matches!(config.core.object_type, 4 | 19) {
        // One MiB per output/source state additionally reserves the analysis
        // transform/windows, float history, synthesis and source candidates,
        // checkpoints, spectral estimates and transactional scratch. Eighteen
        // slots cover all 16 CCE tags plus construction/parse candidates.
        bytes = bytes
            .checked_add((usize::from(config.core.channels) + 18) * 1024 * 1024)
            .ok_or_else(|| invalid("AAC LTP memory estimate overflow"))?;
    }
    // Unknown signalling can admit implicit SBR at a fixed core-rate clock.
    // Optional admission must cover that candidate even before the first FIL.
    if config.sbr_present == Some(true)
        || output_rate != config.core.sample_rate
        || (config.sbr_present.is_none() && matches!(config.core.object_type, 1 | 2 | 3 | 4 | 17 | 19))
    {
        let coupling_states = config.program.as_ref().map_or(0, |p| p.coupling.len());
        if config.core.object_type == 3 {
            // Two retained aligned packet descriptors, nested transactional copies,
            // frequency/grid syntax and quantized/dequantized coefficient vectors.
            // One MiB per channel and configured CCE bounds their small (<64 bands, <=5 envelopes)
            // metadata alongside SSR alignment; checkpoints charge separately.
            bytes = bytes
                .checked_add((usize::from(config.core.channels) + coupling_states) * 1024 * 1024)
                .ok_or_else(|| invalid("SSR SBR metadata memory estimate overflow"))?;
        }
        bytes = bytes
            .checked_add((usize::from(config.core.channels) + coupling_states) * 2 * 1024 * 1024)
            .ok_or_else(|| invalid("AAC memory estimate overflow"))?;
    }
    if config.ps_present != Some(false)
        && config.sbr_present != Some(false)
        && config.core.channels == 1
    {
        // Eight complete fixed PS/bridge state copies cover nested native,
        // syntax, QMF, matrix and decorrelation transactions. Four MiB cover
        // bounded 64-slot/91-band hybrid/matrix rows, pending/future QMF and
        // stereo PCM vectors with Vec growth. Checkpoints are charged separately.
        let fixed = std::mem::size_of::<super::aac_ps_native::NativePsAacDecoder>()
            .checked_add(std::mem::size_of::<super::aac_sbr_ps::Decoder>())
            .and_then(|n| n.checked_mul(8))
            .ok_or_else(|| invalid("PS memory estimate overflow"))?;
        bytes = bytes
            .checked_add(fixed)
            .and_then(|n| n.checked_add(4 * 1024 * 1024))
            .ok_or_else(|| invalid("PS memory estimate overflow"))?;
    }
    Ok(bytes)
}

/// ADTS discovery reserves the SBR DSP even before the first FIL. Disk records
/// use bounded packet/PCM scratch included in the fixed LC I/O reserve.
pub(crate) fn check_adts_decode_admission(asc: &[u8], options: &CopyOptions) -> Result<()> {
    let config = AdtsAudioConfig::parse(asc)?;
    let discovery = matches!(config.core.object_type, 1 | 2 | 3 | 4) && config.sbr_present.is_none();
    let rate = if discovery {
        config
            .core
            .sample_rate
            .checked_mul(2)
            .ok_or_else(|| invalid("AAC output rate overflow"))?
    } else {
        config.output_sample_rate()
    };
    let wave_meta_bytes =
        crate::owned_wave_metadata::wave_metadata_working_bytes(0, options).map_err(|e| invalid(&e))?;
    let estimated = decode_config_admission_bytes(asc, rate)?
        .checked_add(wave_meta_bytes)
        .ok_or_else(|| invalid("AAC memory estimate overflow"))?;
    if options
        .max_controlled_bytes
        .is_some_and(|limit| estimated > limit)
    {
        return Err(invalid(&format!(
            "controlled memory budget exceeded: need {estimated} bytes, limit {}",
            options.max_controlled_bytes.unwrap()
        )));
    }
    Ok(())
}

pub(crate) fn check_decode_admission(channels: u16, options: &CopyOptions) -> Result<()> {
    let Some(limit) = options.max_controlled_bytes else {
        return Ok(());
    };
    let estimated = decode_admission_bytes(channels)?;
    if estimated > limit {
        return Err(invalid(&format!(
            "controlled memory budget exceeded: need {estimated} bytes, limit {limit}"
        )));
    }
    Ok(())
}

/// Decode ADTS without a file index or full PCM buffer. ADTS priming/padding is
/// retained. Ranges use ceil-rounded sample boundaries and retain preroll.
/// Packet counts include preroll; reaching the count/range stops before reading
/// the next header. Full decoding validates the complete source.
///
/// Mono/stereo ADTS negotiates implicit SBR over the selected packet/range
/// prefix before publishing PCM. Encoded packets and core PCM are retained in
/// private temporary storage until SBR is found or the prefix ends. A prefix
/// that ends before any SBR FIL keeps its observed core clock; it never reads
/// past the requested boundary to inspect a later extension. Full LC input is
/// decoded once, with cached PCM copied at EOF; SBR replays the encoded prefix
/// at its selected output clock. Storage is removed on success and error.
///
/// The caller owns publication and flushing; failures can leave partial PCM in
/// its writer. Progress never emits completion for a caller-owned destination.
/// Metadata mutations are rejected by this raw-stream API. The controlled
/// memory policy admits estimated AAC decoder and packet scratch before
/// constructing the decoder; caller-owned readers/writers are excluded.
pub fn decode_adts_pcm<R: Read>(
    source: R,
    output: &mut impl Write,
    interval: Option<(Duration, Duration)>,
    options: &CopyOptions,
) -> Result<AudioDecodeStats> {
    let mut control = CopyProgress::with_adts_admission(options);
    control.check()?;
    if !(options.streams.is_empty() || options.streams == [0]) {
        return Err(invalid("ADTS has only stream 0"));
    }
    if !options.metadata_set.is_empty()
        || !options.metadata_delete.is_empty()
        || !options.stream_metadata_set.is_empty()
        || !options.stream_metadata_delete.is_empty()
    {
        return Err(invalid("raw PCM stream cannot apply metadata mutations"));
    }
    if interval.is_some_and(|(from, to)| from >= to) {
        return Err(invalid("audio interval requires from < to"));
    }
    if let Some(progress) = &options.progress {
        progress.emit(control.event);
    }
    control.check()?;
    if control.packet_limit_reached() {
        return Err(invalid("audio interval contains no samples"));
    }
    let reader = AdtsStreamReader::open_with_packet_limit(
        source,
        options
            .max_packet_bytes
            .min(options.max_controlled_bytes.unwrap_or(usize::MAX)),
    )?;
    decode_adts_aac_reader_controlled(reader, output, interval, &mut control)
}

pub(crate) fn decode_adts_reader_pcm<R: Read>(
    reader: AdtsStreamReader<R>,
    output: &mut impl Write,
    interval: Option<(Duration, Duration)>,
    options: &CopyOptions,
) -> Result<AudioDecodeStats> {
    let mut control = CopyProgress::with_adts_admission(options);
    control.check()?;
    if !(options.streams.is_empty() || options.streams == [0]) {
        return Err(invalid("ADTS has only stream 0"));
    }
    if !options.metadata_set.is_empty()
        || !options.metadata_delete.is_empty()
        || !options.stream_metadata_set.is_empty()
        || !options.stream_metadata_delete.is_empty()
    {
        return Err(invalid("raw PCM stream cannot apply metadata mutations"));
    }
    if interval.is_some_and(|(from, to)| from >= to) {
        return Err(invalid("audio interval requires from < to"));
    }
    if let Some(progress) = &options.progress {
        progress.emit(control.event);
    }
    control.check()?;
    if control.packet_limit_reached() {
        return Err(invalid("audio interval contains no samples"));
    }
    decode_adts_aac_reader_controlled(reader, output, interval, &mut control)
}


pub(crate) fn adts_prefix_info_for_transform<R: Read>(
    source: R,
    interval: Option<(Duration, Duration)>,
    transform: Option<fvid_media_info::AudioDecodeTransform>,
    options: &CopyOptions,
) -> Result<(u32, u16, u32, u64)> {
    let mut quiet = crate::owned_adts_export::pcm_decode_options(options);
    quiet.progress = None;
    let control = CopyProgress::with_adts_admission(&quiet);
    control.check()?;
    if control.packet_limit_reached() {
        return Err(invalid("audio interval contains no samples"));
    }
    let reader = AdtsStreamReader::open_with_packet_limit(
        source,
        quiet
            .max_packet_bytes
            .min(quiet.max_controlled_bytes.unwrap_or(usize::MAX)),
    )?;
    adts_prefix_info_from_reader_for_transform(reader, interval, transform, options)
}

pub(crate) fn adts_prefix_info_from_reader_for_transform<R: Read>(
    reader: AdtsStreamReader<R>,
    interval: Option<(Duration, Duration)>,
    transform: Option<fvid_media_info::AudioDecodeTransform>,
    options: &CopyOptions,
) -> Result<(u32, u16, u32, u64)> {
    let mut quiet = crate::owned_adts_export::pcm_decode_options(options);
    quiet.progress = None;
    let mut control = CopyProgress::with_adts_admission(&quiet);
    control.check()?;
    if control.packet_limit_reached() {
        return Err(invalid("audio interval contains no samples"));
    }
    check_adts_decode_admission(reader.audio_specific_config(), options)?;
    let negotiated = negotiate_adts_aac_reader(reader, interval, &mut control)?;
    let rate = negotiated.sample_rate();
    let channels = negotiated.channels();
    let mask = negotiated.channel_mask();
    let need_frames = (transform.is_none() && options.max_controlled_bytes.is_some())
        || transform.is_some_and(|t| {
            t.interval.is_some()
                || (options.max_controlled_bytes.is_some()
                    && (t.sample_rate.is_some()
                        || t.channels.is_some()
                        || t.volume.is_some()
                        || !options.metadata_set.is_empty()
                        || !options.metadata_delete.is_empty()
                        || !options.stream_metadata_set.is_empty()
                        || !options.stream_metadata_delete.is_empty()))
        });
    let frames = if need_frames {
        let stats = negotiated.decode(&mut std::io::sink(), &mut control)?;
        if transform.is_some_and(|t| t.interval.is_some()) {
            crate::owned_ogg_opus_export::check_nonempty_interval_prefix(
                transform,
                stats.sample_rate,
                stats.sample_frames,
            )
            .map_err(|e| invalid(&e))?;
        }
        stats.sample_frames
    } else {
        1024
    };
    Ok((rate, channels, mask, frames))
}

enum AdtsOutputDecoder {
    Core(AdtsPacketDecoder),
    Ps(AdtsPsDecoder),
}
impl AdtsOutputDecoder {
    fn sample_rate(&self) -> u32 {
        match self {
            Self::Core(d) => d.sample_rate(),
            Self::Ps(d) => d.sample_rate(),
        }
    }
    fn channel_mask(&self) -> u32 {
        match self {
            Self::Core(d) => d.channel_mask(),
            Self::Ps(d) => d.channel_mask(),
        }
    }
    fn decode(&mut self, packet: &[u8]) -> Result<Vec<f32>> {
        match self {
            Self::Core(d) => Ok(d
                .decode_timed(packet, 0, u64::from(d.core_frame_samples()))?
                .map_or_else(Vec::new, |f| f.samples)),
            Self::Ps(d) => Ok(d.decode(packet)?.map_or_else(Vec::new, |f| f.pcm)),
        }
    }
    fn finish(&mut self) -> Result<Option<Vec<f32>>> {
        match self {
            Self::Core(d) => Ok(d.finish()?.map(|f| f.samples)),
            Self::Ps(d) => Ok(d.finish()?.map(|f| f.pcm)),
        }
    }
}

pub fn decode_adts_aac_reader_controlled<R: std::io::Read, C: AudioDecodeControl>(
    reader: AdtsStreamReader<R>,
    output: &mut impl std::io::Write,
    interval: Option<(Duration, Duration)>,
    control: &mut C,
) -> Result<AudioDecodeStats> {
    negotiate_adts_aac_reader(reader, interval, control)?.decode(output, control)
}

/// Negotiate before a caller constructs its resampler or output header. Source
/// packet progress is counted during discovery, never a second time on replay.
pub struct NegotiatedAdts<R: std::io::Read> {
    reader: AdtsStreamReader<R>,
    decoder: AdtsOutputDecoder,
    spool: Option<AdtsClockSpool>,
    cached_lc: bool,
    interval: Option<(Duration, Duration)>,
    channels: u16,
}
impl<R: std::io::Read> NegotiatedAdts<R> {
    pub fn sample_rate(&self) -> u32 {
        self.decoder.sample_rate()
    }
    pub fn channels(&self) -> u16 {
        self.channels
    }
    pub fn channel_mask(&self) -> u32 {
        self.decoder.channel_mask()
    }
    pub fn decode<C: AudioDecodeControl>(
        mut self,
        output: &mut impl std::io::Write,
        control: &mut C,
    ) -> Result<AudioDecodeStats> {
        let mut stats = AudioDecodeStats {
            sample_frames: 0,
            decoded_frames: 0,
            sample_rate: self.sample_rate(),
            channels: self.channels,
        };
        let (from, to) = adts_interval_boundaries(self.interval, stats.sample_rate)?;
        let mut position = 0u64;
        if let Some(mut spool) = self.spool.take() {
            spool.rewind()?;
            let mut packet = Vec::new();
            let mut cached_pcm = Vec::new();
            for _ in 0..spool.records {
                control.check()?;
                if self.cached_lc {
                    spool.read_record(None, Some(&mut cached_pcm))?;
                    emit_adts_samples(&cached_pcm, output, from, to, &mut position, &mut stats)?;
                } else {
                    spool.read_record(Some(&mut packet), None)?;
                    let samples = self.decoder.decode(&packet)?;
                    emit_adts_samples(&samples, output, from, to, &mut position, &mut stats)?;
                }
            }
            // LC discovery ended at EOF or the requested boundary. All of its
            // selected input is already in the spool; never read another header.
            if self.cached_lc {
                if stats.sample_frames == 0 {
                    return Err(invalid("audio interval contains no samples"));
                }
                return Ok(stats);
            }
        }
        while position < to {
            control.check()?;
            if control.packet_limit_reached() {
                break;
            }
            let Some(packet) = self.reader.next_packet()? else {
                break;
            };
            let samples = self.decoder.decode(&packet)?;
            emit_adts_samples(&samples, output, from, to, &mut position, &mut stats)?;
            control.packet(packet.len())?;
        }
        while position < to {
            let Some(frame) = self.decoder.finish()? else {
                break;
            };
            emit_adts_samples(&frame, output, from, to, &mut position, &mut stats)?;
        }
        if stats.sample_frames == 0 {
            return Err(invalid("audio interval contains no samples"));
        }
        Ok(stats)
    }
}

pub fn negotiate_adts_aac_reader<R: std::io::Read, C: AudioDecodeControl>(
    mut reader: AdtsStreamReader<R>,
    interval: Option<(Duration, Duration)>,
    control: &mut C,
) -> Result<NegotiatedAdts<R>> {
    control.check()?;
    if interval.is_some_and(|(from, to)| from >= to) {
        return Err(invalid("audio interval requires from < to"));
    }
    let config = reader.configuration();
    let asc = reader.audio_specific_config().to_vec();
    control.check_admission(&asc)?;
    let parsed = AdtsAudioConfig::parse(&asc)?;
    if matches!(parsed.core.object_type, 1 | 2 | 3 | 4)
        && AdtsPsProbe::accepts_mono_program(&asc)?
        && parsed.ps_present.is_none()
        && parsed.sbr_present.is_none()
    {
        // PS may arrive after an ordinary SBR packet. Scan the selected encoded
        // prefix before publishing channel geometry, retaining only disk records.
        // The syntax probe does not synthesize core/QMF/PS PCM.
        let double_rate = config
            .sample_rate
            .checked_mul(2)
            .ok_or_else(|| invalid("AAC output rate overflow"))?;
        let mut probe = AdtsPsProbe::new(&asc, double_rate)?;
        let mut storage = AdtsClockSpool::new(config.channels)?;
        let mut rate = config.sample_rate;
        let mut position = 0u64;
        let core_to = match interval {
            Some((_, to)) => adts_sample_boundary(to, config.sample_rate)?,
            None => u64::MAX,
        };
        while position < core_to && !control.packet_limit_reached() {
            control.check()?;
            let Some(packet) = reader.next_packet()? else {
                break;
            };
            probe.read(&packet)?;
            // The probe has validated any SBR header/history before the FIL
            // presence publishes the implicit double-rate ADTS output clock.
            if AdtsStreamReader::<R>::packet_has_sbr(&packet, &asc)? {
                rate = double_rate;
            }
            storage.push(&packet, &[])?;
            position = position
                .checked_add(u64::from(parsed.core.frame_samples))
                .ok_or_else(|| invalid("audio position overflow"))?;
            control.packet(packet.len())?;
        }
        let (decoder, channels) = if probe.ps_detected() {
            (
                AdtsOutputDecoder::Ps(AdtsPsDecoder::new_with_in_band_ps(&asc, rate)?),
                2,
            )
        } else {
            (
                AdtsOutputDecoder::Core(AdtsPacketDecoder::new_with_output_rate(&asc, rate)?),
                config.channels,
            )
        };
        return Ok(NegotiatedAdts {
            reader,
            decoder,
            spool: Some(storage),
            cached_lc: false,
            interval,
            channels,
        });
    }

    let discovery = matches!(parsed.core.object_type, 1 | 2 | 3 | 4) && parsed.sbr_present.is_none();
    let mut decoder = if discovery {
        AdtsPacketDecoder::new_with_sbr_detection(&asc)?
    } else {
        AdtsPacketDecoder::new(&asc)?
    };
    let channels = usize::from(config.channels);
    let mut spool = None;
    let mut cached_lc = false;
    if discovery {
        // ADTS has no extension clock. Until a valid SBR FIL or the requested
        // prefix ends, retain encoded packets and core PCM on disk. Never publish
        // a core-rate prefix followed by a double-rate suffix. Core replay copies
        // its cached PCM; SBR replay reconstructs the prefix with QMF history.
        let core_to = match interval {
            Some((_, to)) => adts_sample_boundary(to, config.sample_rate)?,
            None => u64::MAX,
        };
        let mut storage = AdtsClockSpool::new(config.channels)?;
        let mut core_position = 0u64;
        while core_position < core_to && !control.packet_limit_reached() {
            control.check()?;
            let Some(packet) = reader.next_packet()? else {
                break;
            };
            let samples = decoder
                .decode_timed(&packet, 0, u64::from(decoder.core_frame_samples()))?
                .map_or_else(Vec::new, |f| f.samples);
            let frames = (samples.len() / channels) as u64;
            let core_frames = if decoder.sample_rate() == config.sample_rate {
                frames
            } else {
                if decoder.sample_rate()
                    != config
                        .sample_rate
                        .checked_mul(2)
                        .ok_or_else(|| invalid("AAC output rate overflow"))?
                    || !frames.is_multiple_of(2)
                {
                    return Err(invalid("invalid ADTS SBR sample clock"));
                }
                frames / 2
            };
            core_position = core_position
                .checked_add(core_frames)
                .ok_or_else(|| invalid("audio position overflow"))?;
            storage.push(&packet, &samples)?;
            control.packet(packet.len())?;
            if decoder.sample_rate() != config.sample_rate {
                break;
            }
        }
        let rate = decoder.sample_rate();
        cached_lc = rate == config.sample_rate;
        if cached_lc && core_position < core_to {
            // Delayed SSR may retain the final core frame after the last encoded
            // packet. Store its PCM as a tail-only record; this path never replays
            // encoded data and never publishes before negotiation succeeds.
            if let Some(frame) = decoder.finish()? {
                storage.push(&[], &frame.samples)?;
            }
        }
        if !cached_lc {
            // Admission covers one decoder, not two simultaneous histories.
            drop(decoder);
            decoder = AdtsPacketDecoder::new_with_output_rate(&asc, rate)?;
        }
        spool = Some(storage);
    }
    Ok(NegotiatedAdts {
        reader,
        decoder: AdtsOutputDecoder::Core(decoder),
        spool,
        cached_lc,
        interval,
        channels: config.channels,
    })
}

pub fn adts_sample_boundary(time: Duration, rate: u32) -> Result<u64> {
    let ticks = time
        .as_nanos()
        .checked_mul(u128::from(rate))
        .ok_or_else(|| invalid("audio interval overflow"))?;
    u64::try_from(ticks.div_ceil(1_000_000_000)).map_err(|_| invalid("audio interval overflow"))
}

fn adts_interval_boundaries(
    interval: Option<(Duration, Duration)>,
    rate: u32,
) -> Result<(u64, u64)> {
    match interval {
        Some((from, to)) => Ok((
            adts_sample_boundary(from, rate)?,
            adts_sample_boundary(to, rate)?,
        )),
        None => Ok((0, u64::MAX)),
    }
}

fn emit_adts_samples(
    samples: &[f32],
    output: &mut impl std::io::Write,
    from: u64,
    to: u64,
    position: &mut u64,
    stats: &mut AudioDecodeStats,
) -> Result<()> {
    let channels = usize::from(stats.channels);
    let frames = (samples.len() / channels) as u64;
    let end = position
        .checked_add(frames)
        .ok_or_else(|| invalid("audio position overflow"))?;
    let first = from.saturating_sub(*position).min(frames) as usize;
    let last = to.saturating_sub(*position).min(frames) as usize;
    write_pcm_samples(
        output,
        &samples[first * channels..last.max(first) * channels],
    )?;
    stats.sample_frames = stats
        .sample_frames
        .checked_add(last.saturating_sub(first) as u64)
        .ok_or_else(|| invalid("audio sample count overflow"))?;
    if !samples.is_empty() {
        stats.decoded_frames += 1;
    }
    *position = end;
    Ok(())
}

/// Private variable-size records. Retained RAM is one bounded ADTS packet and
/// one admitted PCM block, regardless of prefix length. Close before unlink
/// for Windows; the RAII guard also removes storage after decode/write errors.
struct AdtsClockSpool {
    file: Option<std::fs::File>,
    path: std::path::PathBuf,
    records: u64,
    max_pcm_samples: usize,
}
impl AdtsClockSpool {
    fn new(channels: u16) -> Result<Self> {
        let max_pcm_samples = usize::from(channels)
            .max(2)
            .checked_mul(2048)
            .ok_or_else(|| invalid("ADTS spool PCM extent overflow"))?;
        use std::sync::atomic::{AtomicU64, Ordering};
        static NEXT: AtomicU64 = AtomicU64::new(0);
        loop {
            let path = std::env::temp_dir().join(format!(
                "fvid-adts-clock-{}-{}.spool",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            let mut options = std::fs::OpenOptions::new();
            options.read(true).write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            match options.open(&path) {
                Ok(file) => {
                    return Ok(Self {
                        file: Some(file),
                        path,
                        records: 0,
                        max_pcm_samples,
                    });
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(error.into()),
            }
        }
    }
    fn push(&mut self, packet: &[u8], samples: &[f32]) -> Result<()> {
        use std::io::Write;
        let packet_bytes =
            u32::try_from(packet.len()).map_err(|_| invalid("ADTS packet length overflow"))?;
        let pcm_samples =
            u32::try_from(samples.len()).map_err(|_| invalid("ADTS PCM length overflow"))?;
        if packet.len() > 8191 || samples.len() > self.max_pcm_samples {
            return Err(invalid("ADTS clock spool record exceeds admitted extent"));
        }
        let file = self.file.as_mut().unwrap();
        file.write_all(&packet_bytes.to_le_bytes())?;
        file.write_all(&pcm_samples.to_le_bytes())?;
        file.write_all(packet)?;
        write_pcm_samples(file, samples)?;
        self.records = self
            .records
            .checked_add(1)
            .ok_or_else(|| invalid("ADTS spool count overflow"))?;
        Ok(())
    }
    fn rewind(&mut self) -> Result<()> {
        use std::io::Seek;
        self.file.as_mut().unwrap().rewind()?;
        Ok(())
    }
    fn read_record(
        &mut self,
        packet: Option<&mut Vec<u8>>,
        pcm: Option<&mut Vec<f32>>,
    ) -> Result<()> {
        use std::io::{Read, Seek, SeekFrom};
        let file = self.file.as_mut().unwrap();
        let mut header = [0; 8];
        file.read_exact(&mut header)?;
        let packet_bytes = u32::from_le_bytes(header[..4].try_into().unwrap()) as usize;
        let pcm_samples = u32::from_le_bytes(header[4..].try_into().unwrap()) as usize;
        // ADTS frame_length is 13 bits. One decoded record has at most 2048
        // frames per admitted channel (or stereo for implicit PS).
        if packet_bytes > 8191 || pcm_samples > self.max_pcm_samples {
            return Err(invalid("invalid ADTS clock spool record"));
        }
        if let Some(packet) = packet {
            packet.resize(packet_bytes, 0);
            file.read_exact(packet)?;
        } else {
            file.seek(SeekFrom::Current(packet_bytes as i64))?;
        }
        if let Some(pcm) = pcm {
            pcm.resize(pcm_samples, 0.0);
            let mut bytes = [0u8; 4096];
            for chunk in pcm.chunks_mut(1024) {
                file.read_exact(&mut bytes[..chunk.len() * 4])?;
                for (sample, raw) in chunk.iter_mut().zip(bytes.chunks_exact(4)) {
                    *sample = f32::from_le_bytes(raw.try_into().unwrap());
                }
            }
        } else {
            file.seek(SeekFrom::Current((pcm_samples * 4) as i64))?;
        }
        Ok(())
    }
}
impl Drop for AdtsClockSpool {
    fn drop(&mut self) {
        drop(self.file.take());
        let _ = std::fs::remove_file(&self.path);
    }
}

// Serialization scratch stays on the stack and does not grow with stream length.
fn write_pcm_samples(output: &mut impl std::io::Write, samples: &[f32]) -> Result<()> {
    let mut bytes = [0u8; 4096];
    for chunk in samples.chunks(bytes.len() / 4) {
        for (sample, target) in chunk.iter().zip(bytes.as_chunks_mut::<4>().0.iter_mut()) {
            target.copy_from_slice(&sample.to_le_bytes());
        }
        output.write_all(&bytes[..chunk.len() * 4])?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn pcm_serialization_preserves_bits_and_handles_short_writes() {
        struct ShortWriter {
            bytes: Vec<u8>,
            calls: usize,
        }
        impl std::io::Write for ShortWriter {
            fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
                self.calls += 1;
                let count = bytes.len().min(777);
                self.bytes.extend_from_slice(&bytes[..count]);
                Ok(count)
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        let samples: Vec<f32> = (0..2051).map(|n| f32::from_bits(n * 7919)).collect();
        let expected: Vec<u8> = samples.iter().flat_map(|s| s.to_le_bytes()).collect();
        let mut output = ShortWriter {
            bytes: Vec::new(),
            calls: 0,
        };
        super::write_pcm_samples(&mut output, &samples).unwrap();
        assert_eq!(output.bytes, expected);
        assert!(output.calls < 20);
        let calls = output.calls;
        super::write_pcm_samples(&mut output, &[]).unwrap();
        assert_eq!(output.calls, calls);
    }

    #[test]
    fn standalone_stream_api_retains_exact_clock_and_packet_prefix() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures");
        let bytes = std::fs::read(root.join("playback-errors/aac-packet-prefix.aac")).unwrap();
        let mut output = Vec::new();
        let options = fvid_control::CopyOptions {
            max_packets: Some(3),
            ..Default::default()
        };
        let stats = super::decode_adts_pcm(bytes.as_slice(), &mut output, None, &options).unwrap();
        assert_eq!(
            (
                stats.sample_frames,
                stats.decoded_frames,
                stats.sample_rate,
                stats.channels
            ),
            (3072, 3, 44100, 1)
        );
        assert_eq!(output.len(), 3072 * 4);
        assert!(
            super::decode_adts_pcm(bytes.as_slice(), &mut Vec::new(), None, &Default::default())
                .unwrap_err()
                .to_string()
                .contains("fill whole buffer")
        );
    }
    #[test]
    fn clock_spool_preserves_pcm_bits_and_removes_storage_on_decode_writer_error() {
        let mut spool = super::AdtsClockSpool::new(2).unwrap();
        let path = spool.path.clone();
        let samples = [f32::from_bits(0x7fc01234), -0.0, f32::INFINITY];
        spool.push(&[1, 2, 3], &samples).unwrap();
        spool.rewind().unwrap();
        let mut packet = Vec::new();
        let mut pcm = Vec::new();
        spool
            .read_record(Some(&mut packet), Some(&mut pcm))
            .unwrap();
        assert_eq!(packet, [1, 2, 3]);
        assert_eq!(
            pcm.iter().map(|s| s.to_bits()).collect::<Vec<_>>(),
            samples.iter().map(|s| s.to_bits()).collect::<Vec<_>>()
        );
        drop(spool);
        assert!(!path.exists());

        struct Fails;
        impl std::io::Write for Fails {
            fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
                Err(std::io::Error::other("negotiated sink failed"))
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        for name in [
            "playback-errors/he-aac-delayed-sbr.aac",
            "audio/aac-mono-44k.aac",
        ] {
            let bytes = std::fs::read(
                std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("../../tests/fixtures")
                    .join(name),
            )
            .unwrap();
            let options = fvid_control::CopyOptions::default();
            let mut control = crate::owned_audio_control::CopyProgress::with_adts_admission(&options);
            let reader = super::AdtsStreamReader::open(bytes.as_slice()).unwrap();
            let negotiated = super::negotiate_adts_aac_reader(reader, None, &mut control).unwrap();
            let path = negotiated.spool.as_ref().unwrap().path.clone();
            assert!(path.exists());
            let error = negotiated.decode(&mut Fails, &mut control).unwrap_err();
            assert!(error.to_string().contains("negotiated sink failed"));
            assert!(!path.exists());
        }
    }
}

#[cfg(test)]
mod ltp_admission_tests {
    #[test]
    fn implicit_ltp_sbr_reserves_the_same_dsp_as_explicit_signalling() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/playback-errors");
        let m: serde_json::Value =
            serde_json::from_slice(&std::fs::read(root.join("aac-ltp-sbr.json")).unwrap()).unwrap();
        for rate in [24000u32, 48000] {
            let estimate = |signal: &str| {
                let case = m["cases"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|c| c["rate"] == rate && c["signal"] == signal)
                    .unwrap();
                let asc: Vec<u8> = case["asc"]
                    .as_str()
                    .unwrap()
                    .as_bytes()
                    .chunks_exact(2)
                    .map(|v| u8::from_str_radix(std::str::from_utf8(v).unwrap(), 16).unwrap())
                    .collect();
                super::decode_config_admission_bytes(&asc, rate).unwrap()
            };
            assert_eq!(estimate("implicit"), estimate("explicit"));
            assert_eq!(estimate("sync"), estimate("explicit"));
        }
    }
    #[test]
    fn ltp_asc_reserves_float_analysis_and_transaction_storage_before_decode() {
        // Ordinary mono LTP, 24 kHz, 1024 and 960 samples respectively.
        for asc in [[0x23, 0x08], [0x23, 0x0c]] {
            let parsed = super::super::config::AudioSpecificConfig::parse(&asc).unwrap();
            assert_eq!(parsed.core.object_type, 4);
            let base = super::decode_admission_bytes(1).unwrap();
            let admitted = super::decode_config_admission_bytes(&asc, 24000).unwrap();
            assert!(admitted >= base + 19 * 1024 * 1024);
            let decoder = super::super::NativeAacDecoder::new(&asc).unwrap();
            let checkpoint = decoder.checkpoint();
            assert!(
                decoder
                    .retained_payload_bytes_with_checkpoint(Some(&checkpoint))
                    .unwrap()
                    < admitted
            );
        }
    }
}
