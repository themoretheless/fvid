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
    mut input: adts::StreamReader<R>,
    output: &mut W,
    cancel: Option<&CancelFlag>,
    progress: Option<&ProgressHook>,
    max_packets: Option<u64>,
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
    )
}

fn write_aac_packets<W: Write + Seek>(
    config: adts::Header,
    asc: &[u8],
    mut next_packet: impl FnMut() -> Result<Option<Vec<u8>>>,
    output: &mut W,
    cancel: Option<&CancelFlag>,
    progress: Option<&ProgressHook>,
    max_packets: Option<u64>,
) -> Result<ProgressEvent> {
    let samples = u64::from(AacConfig::parse(asc)?.frame_samples);
    let spec = TrackSpec {
        encoding: Encoding::Aac {
            configuration: asc,
            sample_rate: config.sample_rate,
            channels: config.channels,
        },
        name: "",
        language: "",
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
    let mut writer = PacketWriter::new(output, &[spec])?;
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
        writer.rewrite_adts_rate(asc, rate, config.channels)?;
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
