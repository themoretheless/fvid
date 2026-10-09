//! Sequential AAC-LC/HE-AAC decoding to caller-owned interleaved float32 PCM.
use super::{
    Error, NativeAacDecoder as AdtsPacketDecoder, aac_ps_native::{NativePsAacDecoder as AdtsPsDecoder, InBandPsProbe as AdtsPsProbe}, Result, adts::StreamReader as AdtsStreamReader,
    config::AudioSpecificConfig as AdtsAudioConfig, invalid,
};
use fvid_control::{CopyOptions, ProgressEvent};
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
pub(crate) struct DecodeProgress<'a> {
    options: &'a CopyOptions,
    event: ProgressEvent,
}
impl DecodeProgress<'_> {
    fn check_admission(&self, asc: &[u8]) -> Result<()> {
        check_adts_decode_admission(asc, self.options)
    }
    fn check(&self) -> Result<()> {
        if self
            .options
            .cancel
            .as_ref()
            .is_some_and(|c| c.is_cancelled())
        {
            return Err(invalid("media operation cancelled"));
        }
        crate::owned_budget::check_rss_budget(self.options).map_err(Error)
    }
    fn packet_limit_reached(&self) -> bool {
        self.options
            .max_packets
            .is_some_and(|max| self.event.packets >= max)
    }
    fn packet(&mut self, bytes: usize) -> Result<()> {
        self.event.packets = self
            .event
            .packets
            .checked_add(1)
            .ok_or_else(|| invalid("audio packet count overflow"))?;
        self.event.payload_bytes = self
            .event
            .payload_bytes
            .checked_add(bytes as u64)
            .ok_or_else(|| invalid("audio byte count overflow"))?;
        if let Some(progress) = &self.options.progress {
            progress.emit(self.event);
        }
        self.check()
    }
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
    if config.sbr_present == Some(true) || output_rate != config.core.sample_rate {
        let coupling_states = config.program.as_ref().map_or(0, |p| p.coupling.len());
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
    let discovery = matches!(config.core.object_type, 1 | 2) && config.sbr_present.is_none();
    let rate = if discovery {
        config
            .core
            .sample_rate
            .checked_mul(2)
            .ok_or_else(|| invalid("AAC output rate overflow"))?
    } else {
        config.output_sample_rate()
    };
    let estimated = decode_config_admission_bytes(asc, rate)?;
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
    let mut control = DecodeProgress {
        options,
        event: ProgressEvent {
            packets: 0,
            payload_bytes: 0,
            done: false,
        },
    };
    control.check()?;
    if let Some(progress) = &options.progress {
        progress.emit(control.event);
    }
    control.check()?;
    if control.packet_limit_reached() {
        return Err(invalid("audio interval contains no samples"));
    }
    let reader = AdtsStreamReader::open_with_packet_limit(source, options.max_packet_bytes)?;
    decode_adts_aac_reader_controlled(reader, output, interval, &mut control)
}
/// Read-only clock/layout negotiation for file plans and loudness admission.
/// Uses the same selected prefix and decoder budget as execution, but never
/// emits progress or publishes PCM. SBR can be identified before source EOF.
pub(crate) fn adts_prefix_info<R: Read>(
    source: R,
    interval: Option<(Duration, Duration)>,
    options: &CopyOptions,
) -> Result<(u32, u16, u32)> {
    let mut quiet = crate::owned_adts_export::pcm_decode_options(options);
    quiet.progress = None;
    let mut control = DecodeProgress {
        options: &quiet,
        event: ProgressEvent {
            packets: 0,
            payload_bytes: 0,
            done: false,
        },
    };
    control.check()?;
    if control.packet_limit_reached() {
        return Err(invalid("audio interval contains no samples"));
    }
    let reader = AdtsStreamReader::open_with_packet_limit(source, quiet.max_packet_bytes)?;
    let negotiated = negotiate_adts_aac_reader(reader, interval, &mut control)?;
    Ok((
        negotiated.sample_rate(),
        negotiated.channels(),
        negotiated.channel_mask(),
    ))
}
include!("stream_impl.rs");

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
            let mut control = super::DecodeProgress {
                options: &options,
                event: fvid_control::ProgressEvent {
                    packets: 0,
                    payload_bytes: 0,
                    done: false,
                },
            };
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
