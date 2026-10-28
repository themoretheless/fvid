/// Stream strict ADTS packets into Matroska. Per-packet clusters avoid signed
/// block timestamp overflow at a 1 ns clock. No cue table/index is accumulated.
/// Caller must discard partial output after error and publish only on success.
pub fn write_adts<R: Read, W: Write + Seek>(
    input: adts::StreamReader<R>,
    output: &mut W,
    cancel: Option<&CancelFlag>,
    progress: Option<&ProgressHook>,
) -> Result<ProgressEvent> {
    write_adts_limited(input, output, cancel, progress, None)
}

/// Copy at most the requested total packet count; do not read past the limit.
pub fn write_adts_limited<R: Read, W: Write + Seek>(
    input: adts::StreamReader<R>,
    output: &mut W,
    cancel: Option<&CancelFlag>,
    progress: Option<&ProgressHook>,
    max_packets: Option<u64>,
) -> Result<ProgressEvent> {
    write_adts_with_options(input, output, cancel, progress, max_packets, None)
}

/// Copy at most the requested total packet count with optional Matroska metadata mutations.
pub fn write_adts_with_options<R: Read, W: Write + Seek>(
    mut input: adts::StreamReader<R>,
    output: &mut W,
    cancel: Option<&CancelFlag>,
    progress: Option<&ProgressHook>,
    max_packets: Option<u64>,
    options: Option<&fvid_control::CopyOptions>,
) -> Result<ProgressEvent> {
    let asc = input.audio_specific_config().to_vec();
    write_aac_packets(
        input.configuration(),
        &asc,
        || convert_adts_packet(input.next_packet()),
        output,
        cancel,
        progress,
        max_packets,
        options,
    )
}

/// Append compatible ADTS segments to one Matroska track without transcoding.
pub fn concat_adts<R: Read, W: Write + Seek>(
    readers: Vec<adts::StreamReader<R>>,
    output: &mut W,
    cancel: Option<&CancelFlag>,
    progress: Option<&ProgressHook>,
) -> Result<ProgressEvent> {
    concat_adts_limited(readers, output, cancel, progress, None)
}

/// Apply one packet limit across all compatible segments.
pub fn concat_adts_limited<R: Read, W: Write + Seek>(
    readers: Vec<adts::StreamReader<R>>,
    output: &mut W,
    cancel: Option<&CancelFlag>,
    progress: Option<&ProgressHook>,
    max_packets: Option<u64>,
) -> Result<ProgressEvent> {
    concat_adts_with_options(readers, output, cancel, progress, max_packets, None)
}

/// Apply one packet limit and optional Matroska metadata mutations across compatible segments.
pub fn concat_adts_with_options<R: Read, W: Write + Seek>(
    readers: Vec<adts::StreamReader<R>>,
    output: &mut W,
    cancel: Option<&CancelFlag>,
    progress: Option<&ProgressHook>,
    max_packets: Option<u64>,
    options: Option<&fvid_control::CopyOptions>,
) -> Result<ProgressEvent> {
    let mut sequence = adts::SequenceReader::new(readers)?;
    let asc = sequence.audio_specific_config().to_vec();
    write_aac_packets(
        sequence.configuration(),
        &asc,
        || convert_adts_packet(sequence.next_packet()),
        output,
        cancel,
        progress,
        max_packets,
        options,
    )
}

type AdtsMatroskaMetadata = (
    String,
    String,
    FileMetadata,
    std::collections::BTreeMap<String, String>,
    std::collections::BTreeMap<usize, std::collections::BTreeMap<String, String>>,
);

pub(crate) fn extract_adts_matroska_metadata(
    options: Option<&fvid_control::CopyOptions>,
) -> Result<AdtsMatroskaMetadata> {
    let mut name = String::new();
    let mut language = String::new();
    let mut metadata = FileMetadata::default();
    let mut text = std::collections::BTreeMap::new();
    let mut scoped = std::collections::BTreeMap::<
        usize,
        std::collections::BTreeMap<String, String>,
    >::new();
    if let Some(o) = options {
        if o.metadata_set.len() + o.metadata_delete.len() > 64 {
            return Err(invalid("at most 64 container metadata mutations"));
        }
        if o.stream_metadata_set.len() + o.stream_metadata_delete.len() > 64 {
            return Err(invalid("at most 64 stream metadata mutations"));
        }
        for (key, value) in &o.metadata_set {
            if key.is_empty()
                || key.len() > 128
                || value.len() > 1024
                || key.contains('\0')
                || value.contains('\0')
            {
                return Err(invalid("invalid Matroska container metadata"));
            }
            text.retain(|k: &String, _| !k.eq_ignore_ascii_case(key));
            if !metadata.tags.set(key, value) && !value.is_empty() {
                text.insert(key.to_ascii_uppercase(), value.clone());
            }
        }
        for key in &o.metadata_delete {
            if key.is_empty() || key.len() > 128 || key.contains('\0') {
                return Err(invalid("invalid Matroska container metadata key"));
            }
            metadata.tags.set(key, "");
            text.retain(|k, _| !k.eq_ignore_ascii_case(key));
        }
        for (index, key, value) in &o.stream_metadata_set {
            if *index != 0
                || key.is_empty()
                || key.len() > 128
                || value.len() > 1024
                || key.contains('\0')
                || value.contains('\0')
                || matches!(
                    key.to_ascii_lowercase().as_str(),
                    "rotate" | "stereo_mode" | "alpha_mode"
                )
                || (key.eq_ignore_ascii_case("language") && value.len() > 128)
            {
                return Err(invalid("invalid Matroska stream metadata"));
            }
            if key.eq_ignore_ascii_case("title") {
                name.clone_from(value);
            } else if key.eq_ignore_ascii_case("language") {
                language.clone_from(value);
            } else {
                let entries = scoped.entry(0).or_default();
                entries.retain(|k: &String, _| !k.eq_ignore_ascii_case(key));
                if !value.is_empty() {
                    entries.insert(key.to_ascii_uppercase(), value.clone());
                }
            }
        }
        for (index, key) in &o.stream_metadata_delete {
            if *index != 0
                || key.is_empty()
                || key.len() > 128
                || key.contains('\0')
                || matches!(
                    key.to_ascii_lowercase().as_str(),
                    "rotate" | "stereo_mode" | "alpha_mode"
                )
            {
                return Err(invalid("invalid Matroska stream metadata deletion"));
            }
            if key.eq_ignore_ascii_case("title") {
                name.clear();
            } else if key.eq_ignore_ascii_case("language") {
                language.clear();
            } else if let Some(entries) = scoped.get_mut(&0) {
                entries.retain(|k, _| !k.eq_ignore_ascii_case(key));
            }
        }
    }
    Ok((name, language, metadata, text, scoped))
}

#[allow(clippy::too_many_arguments)]
fn write_aac_packets<W: Write + Seek>(
    config: adts::Header,
    asc: &[u8],
    mut next_packet: impl FnMut() -> Result<Option<Vec<u8>>>,
    output: &mut W,
    cancel: Option<&CancelFlag>,
    progress: Option<&ProgressHook>,
    max_packets: Option<u64>,
    options: Option<&fvid_control::CopyOptions>,
) -> Result<ProgressEvent> {
    let samples = u64::from(AacConfig::parse(asc)?.frame_samples);
    let (name, language, metadata, text, scoped) = extract_adts_matroska_metadata(options)?;
    let spec = TrackSpec {
        encoding: Encoding::Aac {
            configuration: asc,
            sample_rate: config.sample_rate,
            channels: config.channels,
        },
        name: &name,
        language: &language,
    };
    let check = || {
        if cancel.is_some_and(|c| c.is_cancelled()) {
            Err(invalid("media operation cancelled"))
        } else {
            Ok(())
        }
    };
    let time = |packet: u64| -> Result<u64> {
        u64::try_from(
            u128::from(packet) * u128::from(samples) * 1_000_000_000
                / u128::from(config.sample_rate),
        )
        .map_err(|_| invalid("Matroska timestamp overflow"))
    };
    check()?;
    let mut writer =
        PacketWriter::new_with_text_metadata(output, &[spec], &[], &metadata, &text, &scoped)?;
    if let Some(h) = progress {
        h.emit(writer.event());
    }
    let mut negotiated_rate = None;
    loop {
        check()?;
        if max_packets.is_some_and(|limit| writer.event().packets >= limit) {
            break;
        }
        let Some(packet) = next_packet()? else {
            break;
        };
        if negotiated_rate.is_none() {
            negotiated_rate = adts::probe_output_rate(&packet, asc).map_err(|e| invalid(&e.to_string()))?;
        }
        let index = writer.event().packets;
        let next = index
            .checked_add(1)
            .ok_or_else(|| invalid("packet count overflow"))?;
        let start = time(index)?;
        writer.write_packet(0, start, time(next)? - start, true, &packet)?;
        if let Some(h) = progress {
            h.emit(writer.event());
        }
    }
    check()?;
    if let Some(rate) = negotiated_rate {
        writer.rewrite_adts_rate(asc, rate, config.channels, &name, &language)?;
    }
    let event = writer.finish()?;
    check()?;
    Ok(event)
}

// The shared muxer has different error types in the core and media crate.
fn convert_adts_packet<E, F>(
    packet: std::result::Result<Option<Vec<u8>>, E>,
) -> std::result::Result<Option<Vec<u8>>, F>
where
    F: From<E>,
{
    packet.map_err(F::from)
}
