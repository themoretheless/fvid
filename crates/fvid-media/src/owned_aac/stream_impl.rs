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

pub(crate) fn decode_adts_aac_reader_controlled<R: std::io::Read>(
    reader: AdtsStreamReader<R>,
    output: &mut impl std::io::Write,
    interval: Option<(Duration, Duration)>,
    control: &mut DecodeProgress<'_>,
) -> Result<AudioDecodeStats> {
    negotiate_adts_aac_reader(reader, interval, control)?.decode(output, control)
}

/// Negotiate before a caller constructs its resampler or output header. Source
/// packet progress is counted during discovery, never a second time on replay.
pub(crate) struct NegotiatedAdts<R: std::io::Read> {
    reader: AdtsStreamReader<R>,
    decoder: AdtsOutputDecoder,
    spool: Option<AdtsClockSpool>,
    cached_lc: bool,
    interval: Option<(Duration, Duration)>,
    channels: u16,
}
impl<R: std::io::Read> NegotiatedAdts<R> {
    pub(crate) fn sample_rate(&self) -> u32 {
        self.decoder.sample_rate()
    }
    pub(crate) fn channels(&self) -> u16 {
        self.channels
    }
    pub(crate) fn channel_mask(&self) -> u32 {
        self.decoder.channel_mask()
    }
    pub(crate) fn decode(
        mut self,
        output: &mut impl std::io::Write,
        control: &mut DecodeProgress<'_>,
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
        if position < to {
            if let Some(frame) = self.decoder.finish()? {
                emit_adts_samples(&frame, output, from, to, &mut position, &mut stats)?;
            }
        }
        if stats.sample_frames == 0 {
            return Err(invalid("audio interval contains no samples"));
        }
        Ok(stats)
    }
}

pub(crate) fn negotiate_adts_aac_reader<R: std::io::Read>(
    mut reader: AdtsStreamReader<R>,
    interval: Option<(Duration, Duration)>,
    control: &mut DecodeProgress<'_>,
) -> Result<NegotiatedAdts<R>> {
    control.check()?;
    if interval.is_some_and(|(from, to)| from >= to) {
        return Err(invalid("audio interval requires from < to"));
    }
    let config = reader.configuration();
    let asc = reader.audio_specific_config().to_vec();
    control.check_admission(&asc)?;
    let parsed = AdtsAudioConfig::parse(&asc)?;
    if parsed.core.object_type == 2
        && parsed.core.channel_configuration == 1
        && parsed.program.is_none()
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
        let mut storage = AdtsClockSpool::new()?;
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

    let discovery = parsed.core.object_type == 2
        && parsed.sbr_present.is_none()
        && parsed.program.is_none()
        && matches!(parsed.core.channel_configuration, 1 | 2);
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
        // prefix ends, retain encoded packets and LC PCM on disk. Never publish
        // a core-rate prefix followed by a double-rate suffix. LC replay copies
        // its cached PCM; SBR replay reconstructs the prefix with QMF history.
        let core_to = match interval {
            Some((_, to)) => adts_sample_boundary(to, config.sample_rate)?,
            None => u64::MAX,
        };
        let mut storage = AdtsClockSpool::new()?;
        let mut core_position = 0u64;
        while core_position < core_to && !control.packet_limit_reached() {
            control.check()?;
            let Some(packet) = reader.next_packet()? else {
                break;
            };
            let samples = decoder.decode(&packet)?;
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

fn adts_sample_boundary(time: Duration, rate: u32) -> Result<u64> {
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
/// one mono/stereo PCM block, regardless of prefix length. Close before unlink
/// for Windows; the RAII guard also removes storage after decode/write errors.
struct AdtsClockSpool {
    file: Option<std::fs::File>,
    path: std::path::PathBuf,
    records: u64,
}
impl AdtsClockSpool {
    fn new() -> Result<Self> {
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
        // ADTS frame_length is 13 bits; discovery is limited to mono/stereo
        // and two 1024-sample output blocks. Validate private records as well.
        if packet_bytes > 8191 || pcm_samples > 4096 {
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
